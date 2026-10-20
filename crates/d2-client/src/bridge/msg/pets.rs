//! The pet list messages 0x7A PetAction and 0x81 AssignMerc. Fields are
//! read at the offsets states (0x7A: pet GUID u32@9, owner
//! GUID u32@5).

use super::super::dispatch::{HandlerError, Message};
use super::super::skills::{self, Owner, SkillList};
use super::super::world::{ClientWorld, ModelInputs, PetRecord, UnitKey, MONSTER, PET_HIRELING};
use super::Bytes;

/// The `hireling` rows the set-up reads.
pub use d2_sim::world::hirelings::HirelingRows;

/// The set with (pet, owner, type, class).
fn set(w: &mut ClientWorld, pet: u32, owner: u32, pet_type: u8, class: u16) {
    if pet_type == PET_HIRELING {
        // The type-7 record with that pet GUID is freed.
        if let Some(i) = w
            .pets
            .iter()
            .position(|r| r.pet_type == PET_HIRELING && r.pet == pet)
        {
            w.pets.remove(i);
        }
    }
    if let Some(r) = w.pets.iter_mut().find(|r| r.pet == pet) {
        r.pet_type = pet_type;
        r.owner = owner;
        r.class = class;
        r.gone = false;
    } else {
        w.pets.insert(
            0,
            PetRecord {
                class,
                pet_type,
                pet,
                owner,
                f1c: 100,
                gone: false,
                extra: None,
            },
        );
    }
}

/// The remove with (pet): the first record with that
/// pet GUID; the local player's hireling stays with gone := 1.
fn remove(w: &mut ClientWorld, pet: u32) {
    let local = w.local_player.map(|k| k.guid);
    let Some(i) = w.pets.iter().position(|r| r.pet == pet) else {
        return;
    };
    let r = &mut w.pets[i];
    if r.pet_type == PET_HIRELING && Some(r.owner) == local {
        r.gone = true;
    } else {
        w.pets.remove(i);
    }
}

/// 0x7A PetAction.
pub fn pet_action(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    if msg.bytes.len() != 13 {
        return Err(HandlerError::Invalid("0x7A is 13 bytes"));
    }
    let b = Bytes(msg.bytes);
    let pet = b.u32(9)?;
    if b.u8(1)? != 0 {
        set(w, pet, b.u32(5)?, b.u8(2)?, b.u16(3)?);
    } else {
        remove(w, pet);
    }
    Ok(())
}

/// 0x81 AssignMerc.
pub fn assign_merc(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    if msg.bytes.len() != 20 {
        return Err(HandlerError::Invalid("0x81 is 20 bytes"));
    }
    let b = Bytes(msg.bytes);
    let pet = b.u32(8)?;
    set(w, pet, b.u32(4)?, b.u8(1)?, b.u16(2)?);
    let extra = [b.u32(0xC)?, b.u32(0x10)?, 0];
    match w.pets.iter_mut().find(|r| r.pet == pet) {
        Some(r) => r.extra = Some(extra),
        None => return Err(HandlerError::Fatal(0x95)),
    }
    // Type 7 with the monster (1, pet) in S: the client hireling set-up
    // with {hire id u32@0x10 & 0xFFFF, seed u32@0xC}, the
    // dead-flag clear, the light (render
    // state) and mode := 1.
    let key = UnitKey::new(MONSTER, pet);
    if b.u8(1)? == PET_HIRELING && w.units.contains_key(&key) {
        hireling_setup(w, msg.inputs, key, b.u32(0x10)? as u16, b.u32(0xC)?)?;
        if let Some(u) = w.units.get_mut(&key) {
            u.mode = 1;
        }
    }
    Ok(())
}

/// The client hireling set-up with (U, {hire id, seed}):
/// unit flags +0xC4 |= 0x202; the offer of (seed, act of the hire id) at
/// the local player's level (the same routine
/// as the server's hire); the `hireling` row of the offer's id at its
/// level; then each of the row's six skills, in slot order up
/// to the first id outside 1 … skills count − 1, is assigned at its
/// level and its native entry's mode := the slot's `Mode`
/// when below 0x21.
pub(crate) fn hireling_setup(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    hire_id: u16,
    seed: u32,
) -> Result<(), HandlerError> {
    let player_level = w
        .local_player
        .and_then(|p| w.units.get(&p))
        .and_then(|p| p.stats.get(&12).copied())
        .unwrap_or(0);
    let expansion = w.expansion != 0;
    let difficulty = u32::from(w.difficulty);
    let Some(u) = w.units.get_mut(&key) else {
        return Ok(());
    };
    u.flag_2 = Some(true);
    u.flag_200 = true;
    let rows = &inputs.tables.hirelings;
    let act0 = rows.act_of_name(expansion, hire_id);
    let Some(offer) = rows.offer(expansion, player_level, seed, act0, difficulty) else {
        return Ok(());
    };
    let Some(row) = rows.row_at(expansion, offer.id, offer.level) else {
        return Ok(());
    };
    let row = &rows.rows[row];
    // Levels above the row's bracket start, never below 0.
    let d = offer.level.wrapping_sub(row.level).max(0);
    let owner = Owner {
        unit_type: MONSTER,
        class: u.class,
    };
    let skill_rows = &inputs.tables.skills;
    let list = u.skills.get_or_insert_with(SkillList::default);
    for slot in row.skills {
        let Ok(skill) = u16::try_from(slot.skill) else {
            break;
        };
        if skill < 1 || usize::from(skill) >= skill_rows.len() {
            break;
        }
        let mut level =
            (i32::from(slot.lvl_per_lvl).wrapping_mul(d) >> 5).wrapping_add(i32::from(slot.level));
        if level > 0x1F {
            level = 0x20;
        }
        skills::assign(list, skill_rows, owner, skill, level, false)?;
        // The signed byte, compared unsigned (a negative
        // `Mode` leaves the entry's `monanim`).
        let mode = i32::from(slot.mode) as u32;
        if let Some(i) = list.native(skill).filter(|_| mode < 0x21) {
            list.entries[i].mode = mode;
        }
    }
    // The assigns' passive-state parts.
    let fx = std::mem::take(&mut list.fx);
    super::super::passive::apply(w, inputs, key, fx)?;
    Ok(())
}
