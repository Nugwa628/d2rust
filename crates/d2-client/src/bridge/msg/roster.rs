//! The player roster: S→C 0x5B PlayerJoined, 0x5C
//! PlayerLeft, 0x65 PlayerKillCount, 0x82 PortalOwnership and 0x8E
//! CorpseAssign over `ClientWorld::roster` / `roster_inactive`. 0x5B,
//! 0x5C and 0x65 emit `RosterChanged` with the active records (step 9).

use super::super::dispatch::{HandlerError, Message};
use super::super::output::Output;
use super::super::world::{
    ClientWorld, KindData, RosterRecord, UnitKey, MONSTER, OBJECT, PET_TYPE_PASS,
};
use super::Bytes;

fn changed(w: &ClientWorld, msg: &Message<'_>) {
    msg.out.push(Output::RosterChanged {
        roster: w.roster.clone(),
    });
}

/// The NUL-terminated bytes of `b` from `off` (the NUL excluded); a
/// string without a NUL ends with the message.
fn cstr(b: &[u8], off: usize) -> &[u8] {
    let s = b.get(off..).unwrap_or(&[]);
    let n = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    &s[..n]
}

/// `strcpy(record + 0x46 + at, s)`: the string and its NUL.
fn copy_string(strings: &mut Vec<u8>, at: usize, s: &[u8]) {
    let end = at + s.len() + 1;
    if strings.len() < end {
        strings.resize(end, 0);
    }
    strings[at..at + s.len()].copy_from_slice(s);
    strings[at + s.len()] = 0;
}

/// The name of a record up to its NUL.
fn name_of(n: &[u8; 16]) -> &[u8] {
    let k = n.iter().position(|&c| c == 0).unwrap_or(16);
    &n[..k]
}

/// 0x5B PlayerJoined: size u16@1, GUID u32@3, class u8@7, name
/// @8 (16 bytes), u16@0x18 … u16@0x20, string 1 @0x22, string 2 after
/// string 1's NUL.
pub fn player_joined(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    let size = usize::from(b.u16(1)?);
    if size != msg.bytes.len() || size < 34 {
        return Err(HandlerError::Invalid("0x5B size word"));
    }
    let guid = b.u32(3)?;
    if guid == u32::MAX {
        return Err(HandlerError::Fatal(0xB3));
    }
    let mut name = [0u8; 16];
    name.copy_from_slice(b.slice(8, 16)?);
    let s1 = cstr(msg.bytes, 0x22);
    let s2 = cstr(msg.bytes, 0x22 + s1.len() + 1);
    // Found by GUID or by name (active list: GUID +0x10
    // equal, or the name +0x00 equal byte for byte up to the NUL),
    // updated in place.
    let found = w
        .roster
        .iter()
        .position(|r| r.guid == guid || name_of(&r.name) == name_of(&name));
    let (mut rec, at) = match found {
        Some(i) => (w.roster.remove(i), Some(i)),
        None => match w.roster_inactive.iter().position(|r| r.guid == guid) {
            Some(i) => (w.roster_inactive.remove(i), None),
            None => (RosterRecord::default(), None),
        },
    };
    // U16@0x1C is not stored.
    rec.name = name;
    rec.guid = guid;
    rec.class = u32::from(b.u8(7)?);
    rec.f20 = b.u16(0x18)?;
    rec.f22 = b.u16(0x1A)?;
    rec.f30 = b.u16(0x1E)?;
    rec.f44 = b.u16(0x20)?;
    copy_string(&mut rec.strings, 0, s1);
    copy_string(&mut rec.strings, 4, s2);
    match at {
        Some(i) => w.roster.insert(i, rec),
        // A new or moved record is prepended.
        None => w.roster.insert(0, rec),
    }
    changed(w, msg);
    Ok(())
}

/// 0x5C PlayerLeft: GUID u32@1; the active record with that
/// GUID (not the corpse list) is freed; `RosterChanged` either way.
pub fn player_left(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 5 {
        return Err(HandlerError::Invalid("0x5C is 5 bytes"));
    }
    let guid = b.u32(1)?;
    if guid == u32::MAX {
        return Err(HandlerError::Fatal(0x121));
    }
    if let Some(i) = w.roster.iter().position(|r| r.guid == guid) {
        w.roster.remove(i);
    }
    changed(w, msg);
    Ok(())
}

/// 0x65 PlayerKillCount: GUID u32@1, count u16@5 (sign-extended).
pub fn player_kill_count(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 7 {
        return Err(HandlerError::Invalid("0x65 is 7 bytes"));
    }
    let Some(i) = w.roster_find(b.u32(1)?) else {
        return Ok(());
    };
    w.roster[i].kills = i32::from(b.u16(5)? as i16);
    changed(w, msg);
    Ok(())
}

/// 0x82 PortalOwnership: owner GUID u32@1, owner name @5–@20,
/// portal GUIDs u32@21, u32@25.
pub fn portal_ownership(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 29 {
        return Err(HandlerError::Invalid("0x82 is 29 bytes"));
    }
    let mut name = [0u8; 16];
    name.copy_from_slice(b.slice(5, 16)?);
    let (a, c) = (b.u32(21)?, b.u32(25)?);
    let set_name = |w: &mut ClientWorld, guid: u32| {
        if let Some(KindData::Object(d)) = w
            .units
            .get_mut(&UnitKey::new(OBJECT, guid))
            .map(|u| &mut u.kind)
        {
            d.owner_name = Some(name);
        }
    };
    set_name(w, a);
    if c == u32::MAX {
        return Ok(());
    }
    set_name(w, c);
    if let Some(i) = w.roster_find(b.u32(1)?) {
        w.roster[i].portals = (a, c);
    }
    Ok(())
}

/// 0x8E CorpseAssign: u8@1, player GUID u32@2, corpse GUID
/// u32@6. No UI refresh.
pub fn corpse_assign(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 10 {
        return Err(HandlerError::Invalid("0x8E is 10 bytes"));
    }
    let (player, corpse) = (b.u32(2)?, b.u32(6)?);
    if b.u8(1)? != 0 {
        let rec = match w.roster_find(player) {
            Some(i) => &mut w.roster[i],
            None => {
                w.roster_inactive.insert(
                    0,
                    RosterRecord {
                        guid: player,
                        ..RosterRecord::default()
                    },
                );
                &mut w.roster_inactive[0]
            }
        };
        if !rec.corpses.contains(&corpse) {
            rec.corpses.insert(0, corpse);
        }
    } else if let Some(i) = w.roster_find(player) {
        let list = &mut w.roster[i].corpses;
        if let Some(k) = list.iter().position(|&c| c == corpse) {
            list.remove(k);
        }
    }
    Ok(())
}

/// 0x75 PlayerPartyInfo (13 bytes): GUID u32@1, party u16@5,
/// level u16@7, u16@9 (not stored), u16@11.
pub fn player_party_info(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let b = Bytes(msg.bytes);
    if msg.bytes.len() != 13 {
        return Err(HandlerError::Invalid("0x75 is 13 bytes"));
    }
    let guid = b.u32(1)?;
    let Some(i) = w.roster_find(guid) else {
        return Ok(());
    };
    // Found as a corpse holder only: nothing.
    if w.roster[i].guid != guid {
        return Ok(());
    }
    let r = &mut w.roster[i];
    r.f22 = b.u16(5)?;
    r.f20 = b.u16(7)?;
    r.f30 = b.u16(11)?;
    pet_pass(w);
    changed(w, msg);
    Ok(())
}

/// With (A, U): h = 0 when the pet of
/// monster GUID `g` is A's own, a party member's, or a listed player's
/// without relation bit 8. `q` is the FIRST pet record with pet GUID `g`
/// (any type). A none -> 1. The relation list node is
/// never present: the relation messages (0x8B / 0x8C) are out of scope,
/// so "no node" -> 0 -> h = 0 after the party
/// test.
fn pet_hostile(w: &ClientWorld, a: Option<u32>, g: u32) -> bool {
    let Some(a) = a else {
        return true;
    };
    let Some(q) = w.pets.iter().find(|p| p.pet == g) else {
        // q none: both would have to be monsters (A is a player): 1.
        return true;
    };
    if q.owner == a {
        return false;
    }
    // The relation of A to the owner.
    let (Some(ra), Some(rb)) = (w.roster_find(a), w.roster_find(q.owner)) else {
        return true;
    };
    let (pa, pb) = (w.roster[ra].f22, w.roster[rb].f22);
    pa != pb || pa == 0xFFFF
}

/// The pet pass: for each type-4 pet record (in
/// list order) whose monster is present, the palette shift index t = 1
/// when with (A, U) = 0 (A = the local player, read once), else
/// 0; render state, kept in `pet_palette`.
fn pet_pass(w: &mut ClientWorld) {
    let local = w.local_player.map(|k| k.guid);
    let levels: Vec<(UnitKey, u8)> = w
        .pets
        .iter()
        .filter(|p| p.pet_type == PET_TYPE_PASS)
        .map(|p| (UnitKey::new(MONSTER, p.pet), p.pet))
        .filter(|(k, _)| w.units.contains_key(k))
        .map(|(k, g)| (k, u8::from(!pet_hostile(w, local, g))))
        .collect();
    w.pet_palette.extend(levels);
}
