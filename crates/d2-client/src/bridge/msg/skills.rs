//! The skill messages: 0x94 BaseSkillLevels (the native skills at
//! join), 0x21 UpdateItemOSkill (one skill's hard-point level), 0x22
//! UpdateItemSkill (a tome / scroll quantity) and 0x23 SetSkill (left /
//! right skill), over the client skill list of [`super::super::skills`];
//! 0x93 (level bonus by element and page), and the skill events 0x99,
//! 0x9A, 0xA3, 0xA5 (outputs for the client effect layer).

use super::super::dispatch::{HandlerError, Message};
use super::super::output::{Output, SkillTarget};
use super::super::passive;
use super::super::skills::{self, Owner, SkillEntry, SkillError, SkillList, NATIVE};
use super::super::world::{ClientWorld, SkillDescRow, SkillRow, UnitKey, PLAYER};
use super::states::state_bit_off;
use super::Bytes;
use d2_sim::skills::LEVEL_CAP_114D;

impl From<SkillError> for HandlerError {
    fn from(e: SkillError) -> Self {
        match e {
            SkillError::BadSkill(_) => HandlerError::Fatal(0x668),
            SkillError::Dangling => {
                HandlerError::Invalid("skill list: a hand references the removed entry")
            }
        }
    }
}

/// The unit `key` in set S with its skill list; `None` when the unit is
/// not in S. A unit without a list gives `Some((_, None))`: every
/// operation then does nothing.
fn unit_list(w: &mut ClientWorld, key: UnitKey) -> Option<(Owner, Option<&mut SkillList>)> {
    let u = w.units.get_mut(&key)?;
    let owner = Owner {
        unit_type: u.key.unit_type,
        class: u.class,
    };
    Some((owner, u.skills.as_mut()))
}

fn rows<'a>(msg: &Message<'a>) -> &'a [SkillRow] {
    &msg.inputs.tables.skills
}

/// 0x94 BaseSkillLevels: count n u8@1, player GUID u32@2, n entries
/// (skill u16, level u8) from @6; each assigned with remove 0.
pub fn base_skill_levels(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    let n = usize::from(b.u8(1)?);
    if msg.bytes.len() != 6 + 3 * n {
        return Err(HandlerError::Invalid("0x94 is 6 + 3 n bytes"));
    }
    let key = UnitKey::new(PLAYER, b.u32(2)?);
    match unit_list(w, key) {
        Some((_, Some(_))) => {}
        _ => return Ok(()),
    }
    // Every entry is assigned (with its passive-state parts); the
    // first error is reported after the loop, as the base game runs them
    // all.
    let mut first: Result<(), HandlerError> = Ok(());
    for i in 0..n {
        let at = 6 + 3 * i;
        let (skill, level) = (b.u16(at)?, b.u8(at + 2)?);
        let (owner, list) = unit_list(w, key).expect("checked above");
        let list = list.expect("checked above");
        let r = skills::assign(list, rows(msg), owner, skill, i32::from(level), false)
            .map_err(HandlerError::from);
        let fx = std::mem::take(&mut list.fx);
        let r = r.and(passive::apply(w, msg.inputs, key, fx));
        if first.is_ok() {
            first = r;
        }
    }
    first
}

/// 0x21 UpdateItemOSkill: unit type u8@1, remove u8@2, GUID u32@3,
/// skill u16@7, base level u8@9; then the skill-tree flag
/// := 0. Bytes @10 and @11 are not read.
pub fn update_item_oskill(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 12 {
        return Err(HandlerError::Invalid("0x21 is 12 bytes"));
    }
    let key = UnitKey::new(b.u8(1)?, b.u32(3)?);
    let (remove, skill, level) = (b.u8(2)? != 0, b.u16(7)?, b.u8(9)?);
    let Some((owner, list)) = unit_list(w, key) else {
        return Ok(());
    };
    let result = match list {
        Some(list) => {
            let r = skills::assign(list, rows(msg), owner, skill, i32::from(level), remove)
                .map_err(HandlerError::from);
            let fx = std::mem::take(&mut list.fx);
            r.and(passive::apply(w, msg.inputs, key, fx))
        }
        None => Ok(()),
    };
    // runs after the assign returns.
    w.skill_tree_flag = Some(0);
    result
}

/// 0x22 UpdateItemSkill: flag u8@11 ≠ 0 → nothing; else the player
/// (0, GUID u32@3)'s native entry of skill u16@7 gets quantity u8@9;
/// no such entry is fatal 0xAD6.
pub fn update_item_skill(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 12 {
        return Err(HandlerError::Invalid("0x22 is 12 bytes"));
    }
    if b.u8(11)? != 0 {
        return Ok(());
    }
    let key = UnitKey::new(PLAYER, b.u32(3)?);
    let (skill, quantity) = (b.u16(7)?, b.u8(9)?);
    let Some((_, list)) = unit_list(w, key) else {
        return Ok(());
    };
    let Some(list) = list else {
        return Ok(());
    };
    let i = list.native(skill).ok_or(HandlerError::Fatal(0xAD6))?;
    list.entries[i].quantity = i32::from(quantity);
    Ok(())
}

/// 0x23 SetSkill: the player (0, GUID u32@2) selects (skill u16@7,
/// owner u32@9) as its left skill when hand u8@6 ≠ 0, else as its right.
pub fn set_skill(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 13 {
        return Err(HandlerError::Invalid("0x23 is 13 bytes"));
    }
    let key = UnitKey::new(PLAYER, b.u32(2)?);
    let (left, skill, owner) = (b.u8(6)? != 0, b.u16(7)?, b.u32(9)?);
    let Some((_, list)) = unit_list(w, key) else {
        return Ok(());
    };
    let Some(list) = list else {
        return Ok(());
    };
    skills::select(list, rows(msg), left, skill, owner)?;
    Ok(())
}

/// `bonus_level(unit, skill)` over
/// the model's stat totals.
fn bonus_level(
    w: &ClientWorld,
    key: UnitKey,
    rows: &[SkillRow],
    desc: &[SkillDescRow],
    e: &SkillEntry,
) -> i32 {
    let Some(r) = rows.get(usize::from(e.skill)) else {
        return 0;
    };
    let Some(u) = w.units.get(&key) else {
        return 0;
    };
    let total = |stat: u16, layer: u16| w.total(key, stat, layer);
    let skill = e.skill;
    let shrine = if u.states.contains(&134) { 2 } else { 0 };
    let mut b = e.level_bonus + shrine + total(127, 0);
    if key.unit_type == PLAYER && u.class as i32 == i32::from(r.charclass) {
        let class = u.class as u16;
        b += total(83, class);
        let page = desc.get(usize::from(r.skilldesc)).map_or(0, |d| d.page);
        if page != 0 {
            b += total(188, (i32::from(page) + 8 * i32::from(class) - 1) as u16);
        }
        b += total(97, skill).min(3);
    } else if key.unit_type == PLAYER {
        b += total(97, skill);
    } else {
        let n = total(97, skill);
        if e.base <= 0 {
            b += n;
        } else if n != 0 {
            b += n.min(3);
        }
    }
    if r.etype != 0 {
        b += total(126, u16::from(r.etype));
    }
    b + total(107, skill)
}

/// `skill_level(unit, entry, 1)`
/// clamped to `0 ≤ L ≤ cap`: cap = `experience.txt` `MaxLvl`,
/// Amazon column (99 in the base game, [`LEVEL_CAP_114D`]).
pub(crate) fn level_with_bonuses(
    w: &ClientWorld,
    key: UnitKey,
    rows: &[SkillRow],
    desc: &[SkillDescRow],
    e: &SkillEntry,
) -> i32 {
    let mut l = e.base;
    if e.owner == NATIVE {
        l += bonus_level(w, key, rows, desc, e);
    }
    l.clamp(0, LEVEL_CAP_114D)
}

/// 0x93: player GUID u32@1, bonus u8@5 (> 0x80 → − 0x100), element
/// u8@6, page u8@7: a level bonus on the qualifying native entries, then
/// the passive refresh.
pub fn skill_bonus(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 8 {
        return Err(HandlerError::Invalid("0x93 is 8 bytes"));
    }
    let key = UnitKey::new(PLAYER, b.u32(1)?);
    let raw = i32::from(b.u8(5)?);
    let bonus = if raw > 0x80 { raw - 0x100 } else { raw };
    let (element, page) = (b.u8(6)?, b.u8(7)?);
    if !w.units.contains_key(&key) {
        return Ok(());
    }
    if bonus == 0 {
        return Err(HandlerError::Fatal(0x96B));
    }
    if w.units[&key].skills.is_none() {
        return Err(HandlerError::Fatal(0x96D));
    }
    let t = &msg.inputs.tables;
    let (rows, desc) = (&t.skills[..], &t.skilldesc[..]);
    let unit_class = w.units[&key].class;
    let mut first: Result<(), HandlerError> = Ok(());
    let mut i = 0;
    while let Some(e) = w.units[&key]
        .skills
        .as_ref()
        .and_then(|l| l.entries.get(i))
        .copied()
    {
        let qualifies = rows.get(usize::from(e.skill)).is_some_and(|r| {
            desc.get(usize::from(r.skilldesc)).is_some_and(|d| {
                r.enhanceable
                    && (element == 0 || r.etype == element)
                    && (page == 4 || i32::from(d.page) == i32::from(page) + 1)
            })
        }) && e.owner == NATIVE
            && level_with_bonuses(w, key, rows, desc, &e) > 0;
        if !qualifies {
            i += 1;
            continue;
        }
        // The native entry exists (E itself).
        let list = w.units.get_mut(&key).and_then(|u| u.skills.as_mut());
        let list = list.expect("checked above");
        // The level with bonus of skill s: a negative result is
        // clamped to 0.
        let owner = Owner {
            unit_type: key.unit_type,
            class: unit_class,
        };
        if let Err(e) = skills::add_bonus(list, rows, owner, e.skill, bonus) {
            if first.is_ok() {
                first = Err(e.into());
            }
        }
        let now = list.entries[i];
        if level_with_bonuses(w, key, rows, desc, &now) == 0 {
            let list = w.units.get_mut(&key).and_then(|u| u.skills.as_mut());
            let list = list.expect("checked above");
            let r = skills::remove(list, rows, e.skill, true).map_err(HandlerError::from);
            // The passive state off and refresh owed to the unit.
            let fx = std::mem::take(&mut list.fx);
            let r = r.and(passive::apply(w, msg.inputs, key, fx));
            if first.is_ok() {
                first = r;
            }
            continue;
        }
        i += 1;
    }
    // Step 4:, the passive skills the unit has whose state
    // is on are refreshed.
    let r = passive::refresh_all(w, msg.inputs, key);
    first.and(r)
}

/// 0x99: unit type u8@1, GUID u32@2, skill u16@6, level u8@8,
/// target type u8@9, target GUID u32@10, w u16@14; both units in S →
/// one `SkillEvent`.
pub fn skill_event_unit(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 16 {
        return Err(HandlerError::Invalid("0x99 is 16 bytes"));
    }
    let unit = UnitKey::new(b.u8(1)?, b.u32(2)?);
    let target = UnitKey::new(b.u8(9)?, b.u32(10)?);
    if !w.units.contains_key(&unit) || !w.units.contains_key(&target) {
        return Ok(());
    }
    msg.out.push(Output::SkillEvent {
        unit,
        skill: b.u16(6)?,
        level: b.u8(8)?,
        target: SkillTarget::Unit(target),
        w: b.u16(14)?,
    });
    Ok(())
}

/// 0x9A: unit type u8@1, GUID u32@2, skill (low u16 of u32@6),
/// level u8@10, x u16@11, y u16@13, w u16@15; the unit in S and x, y ≠
/// 0 → one `SkillEvent`.
pub fn skill_event_point(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 17 {
        return Err(HandlerError::Invalid("0x9A is 17 bytes"));
    }
    let unit = UnitKey::new(b.u8(1)?, b.u32(2)?);
    let (x, y) = (b.u16(11)?, b.u16(13)?);
    if !w.units.contains_key(&unit) || x == 0 || y == 0 {
        return Ok(());
    }
    msg.out.push(Output::SkillEvent {
        unit,
        skill: b.u16(6)?,
        level: b.u8(10)?,
        target: SkillTarget::Point(x, y),
        w: b.u16(15)?,
    });
    Ok(())
}

/// 0xA3: v u8@1, skill u16@2, level i16@4, unit type u8@6, GUID
/// u32@7, target type u8@0xB, target GUID u32@0xC, x u32@0x10, y
/// u32@0x14; the unit in S and the skill in the table → one `SkillDo`.
pub fn skill_do(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 24 {
        return Err(HandlerError::Invalid("0xA3 is 24 bytes"));
    }
    let unit = UnitKey::new(b.u8(6)?, b.u32(7)?);
    let target = UnitKey::new(b.u8(0xB)?, b.u32(0xC)?);
    let skill = b.u16(2)?;
    if !w.units.contains_key(&unit) || usize::from(skill) >= rows(msg).len() {
        return Ok(());
    }
    msg.out.push(Output::SkillDo {
        unit,
        target: w.units.contains_key(&target).then_some(target),
        skill,
        level: b.u16(4)? as i16,
        x: b.u32(0x10)?,
        y: b.u32(0x14)?,
        v: b.u8(1)?,
    });
    Ok(())
}

/// 0xA5: unit type u8@1, GUID u32@2, skill u16@6: a `SkillEndFx`
/// for `srvdofunc` 67, 76, 77, 78, then state 18 off on the unit. 76
/// (Whirlwind's end) also sets E-flag bit 1 of the
/// unit's used entry when bit 0 is set.
pub fn skill_end(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 8 {
        return Err(HandlerError::Invalid("0xA5 is 8 bytes"));
    }
    let unit = UnitKey::new(b.u8(1)?, b.u32(2)?);
    let skill = b.u16(6)?;
    let Some(r) = rows(msg).get(usize::from(skill)) else {
        return Ok(());
    };
    if !w.units.contains_key(&unit) {
        return Ok(());
    }
    if r.srvdofunc == 76 {
        if let Some(e) = super::super::skills::used_entry_mut(w, unit) {
            if e.flags & 1 != 0 {
                e.flags |= 2;
            }
        }
    }
    if matches!(r.srvdofunc, 67 | 76 | 77 | 78) {
        msg.out.push(Output::SkillEndFx {
            unit,
            skill,
            srvdofunc: r.srvdofunc,
        });
    }
    state_bit_off(w, unit, 18);
    Ok(())
}
