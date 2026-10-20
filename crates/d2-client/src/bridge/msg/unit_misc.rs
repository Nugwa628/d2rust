//! The other unit messages, general handlers that act at receive:
//! overlays (0x11), monster enchants (0x57), portal
//! flags and state (0x5F, 0x60), the client missile (0x73), the corpse
//! assign (0x74), the act COF (0x7E), monster data +0x40 (0x98), the
//! monster preload (0xA4) and the NPC heal (0xAB). 0x09 is
//! `super::units::assign_level_warp`.

use super::super::dispatch::{HandlerError, Message};
use super::super::output::Output;
use super::super::world::{ClientWorld, KindData, UnitKey, MONSTER, OBJECT, PLAYER};
use super::stats_items::ItemHeader;
use super::Bytes;
use crate::rules::lighting::environment::act_index;

fn len(msg: &Message<'_>, n: usize, what: &'static str) -> Result<(), HandlerError> {
    if msg.bytes.len() == n {
        Ok(())
    } else {
        Err(HandlerError::Invalid(what))
    }
}

/// 0x11: type u8@1, GUID u32@2, overlay u16@6. One
/// `UnitOverlay` output when the unit is in S and the overlay is below
/// the overlay count.
pub fn unit_overlay(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 8, "0x11 is 8 bytes")?;
    let b = Bytes(msg.bytes);
    let key = UnitKey::new(b.u8(1)?, b.u32(2)?);
    let overlay = b.u16(6)?;
    if !w.units.contains_key(&key) || u32::from(overlay) >= msg.inputs.tables.overlay_count {
        return Ok(());
    }
    let sound = match overlay {
        151 => 396,
        152 => 397,
        _ => 0,
    };
    msg.out.push(Output::UnitOverlay {
        unit: key,
        overlay,
        mode: 2,
        sound,
    });
    Ok(())
}

/// 0x57 NpcEnchants: GUID u32@1, u8@5, name u16@6, umods u8@8,
/// u8@9, u8@0xA, u16@0xC.
pub fn npc_enchants(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 14, "0x57 is 14 bytes")?;
    let b = Bytes(msg.bytes);
    let key = UnitKey::new(MONSTER, b.u32(1)?);
    if b.u8(5)? != 1 {
        return Ok(());
    }
    let Some(KindData::Monster(d)) = w.units.get_mut(&key).map(|u| &mut u.kind) else {
        return Ok(());
    };
    d.name_seed = b.u16(6)?;
    d.umods[0] = b.u8(8)?;
    d.umods[1] = b.u8(9)?;
    d.umods[2] = b.u8(0xA)?;
    if b.u16(0xC)? != 0 {
        d.flags |= 4;
    }
    d.flags |= 8;
    msg.out.push(Output::UmodFx {
        unit: key,
        umods: d.umods,
        flag8: d.flags & 8 != 0,
    });
    Ok(())
}

/// 0x5F PortalFlags: the local player's player data +0x2C :=
/// u32@1.
pub fn portal_flags(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 5, "0x5F is 5 bytes")?;
    let v = Bytes(msg.bytes).u32(1)?;
    let key = w
        .local_player
        .filter(|k| w.units.contains_key(k))
        .ok_or(HandlerError::Fatal(0xD15))?;
    match w.units.get_mut(&key).map(|u| &mut u.kind) {
        Some(KindData::Player(p)) => {
            p.f2c = v;
            Ok(())
        }
        _ => Err(HandlerError::Fatal(0xFC0)),
    }
}

/// 0x60 TownPortalState: portal flags u8@1, level u8@2, object
/// GUID u32@3.
pub fn town_portal_state(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 7, "0x60 is 7 bytes")?;
    let b = Bytes(msg.bytes);
    if w.local().is_none() {
        return Err(HandlerError::Fatal(0xD21));
    }
    let key = UnitKey::new(OBJECT, b.u32(3)?);
    let Some(u) = w.units.get_mut(&key) else {
        return Ok(());
    };
    if !matches!(u.class, 59 | 60) {
        return Err(HandlerError::Fatal(0x560));
    }
    let KindData::Object(d) = &mut u.kind else {
        return Err(HandlerError::Invalid("0x60: a portal without object data"));
    };
    d.interact = b.u8(2)?;
    d.portal_flags |= b.u8(1)? & 3;
    Ok(())
}

/// 0x73: a client-only missile (set C, not the model): one
/// `ClientMissile` output with the local player's key and the fields.
pub fn client_missile(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 32, "0x73 is 32 bytes")?;
    let b = Bytes(msg.bytes);
    msg.out.push(Output::ClientMissile {
        owner: w.local_player.filter(|k| w.units.contains_key(k)),
        class: b.u16(5)?,
        f07: b.u32(7)?,
        f0b: b.u32(0xB)?,
        f0f: b.u32(0xF)?,
        f13: b.u32(0x13)?,
        f17: b.u16(0x17)?,
        source: UnitKey::new(b.u8(0x19)?, b.u32(0x1A)?),
        f1e: b.u8(0x1E)?,
        f1f: b.u8(0x1F)?,
    });
    Ok(())
}

/// 0x74 PlayerCorpseAssign: u8@1, player GUID u32@2, corpse GUID
/// u32@6.
pub fn player_corpse_assign(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 10, "0x74 is 10 bytes")?;
    let b = Bytes(msg.bytes);
    let p = UnitKey::new(PLAYER, b.u32(2)?);
    if b.u8(1)? == 0 || !w.units.contains_key(&p) {
        return Ok(());
    }
    // Step 1: mode 0.
    if !w.units[&p].is_dead() {
        w.units.get_mut(&p).expect("checked above").mode = 0;
    }
    // Step 2: the corpse placed at P's position, in P's room, then its
    // path direction := P's.
    if w.local_player == Some(p) {
        let k = UnitKey::new(PLAYER, b.u32(6)?);
        if w.units.contains_key(&k) {
            let at = w.units[&p].position;
            let room = w.room_units.room_of(p);
            if let Some(u) = w.units.get_mut(&k) {
                u.position = at;
                u.direction_of = Some(p);
            }
            if w.active_rooms.is_some() {
                w.room_units.place(k, room);
            }
        }
    }
    // Step 3: P's body items are taken off and their item units removed
    // from S: the model holds
    // no inventory nodes, so the body items are the item units whose last
    // record (0x9C / 0x9D) names P with header mode 1, in GUID order.
    // Measured (client state of a recorded death on the moor):
    // at the 0x74 of frame 47 the worn cap and
    // short sword (GUIDs 1, 2) leave the base game's S and the inventory potion
    // (3) stays, the same in D2RUST.
    let body: Vec<UnitKey> = w
        .units
        .values()
        .filter(|u| match &u.kind {
            KindData::Item(d) => d.last.as_ref().is_some_and(|r| {
                r.owner == Some(p) && ItemHeader::peek(&r.stream).is_some_and(|h| h.mode == 1)
            }),
            _ => false,
        })
        .map(|u| u.key)
        .collect();
    for k in body {
        w.remove(k);
    }
    Ok(())
}

/// 0x7E: the message bytes are not read; one `CommonCof` with
/// the act index of the local player's room level (level 0 without a
/// room).
pub fn common_cof(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 5, "0x7E is 5 bytes")?;
    let level = w.player_level().map_or(0, u32::from);
    msg.out.push(Output::CommonCof {
        act: act_index(level),
    });
    Ok(())
}

/// 0x98: GUID u32@1, u16@5: monster data +0x40.
pub fn monster_f40(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 7, "0x98 is 7 bytes")?;
    let b = Bytes(msg.bytes);
    let v = b.u16(5)?;
    let key = UnitKey::new(MONSTER, b.u32(1)?);
    if let Some(KindData::Monster(d)) = w.units.get_mut(&key).map(|u| &mut u.kind) {
        d.f40 = Some(if v == 0xFFFF { -1 } else { i32::from(v) });
    }
    Ok(())
}

/// 0xA4 BaalWave: class u16@1 below the monstats count → one
/// `MonsterPreload`.
pub fn baal_wave(_: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 3, "0xA4 is 3 bytes")?;
    let class = Bytes(msg.bytes).u16(1)?;
    if usize::from(class) < msg.inputs.tables.monsters.len() {
        msg.out.push(Output::MonsterPreload { class });
    }
    Ok(())
}

/// 0xAB NpcHeal: type u8@1, GUID u32@2, life u8@6 (of 128).
/// A unit with flag 0x200 (the hireling setup of 0x81)
/// is skipped.
pub fn npc_heal(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 7, "0xAB is 7 bytes")?;
    let b = Bytes(msg.bytes);
    let key = UnitKey::new(b.u8(1)?, b.u32(2)?);
    let life = b.u8(6)?;
    if !w.units.get(&key).is_some_and(|u| !u.flag_200) || w.local_player == Some(key) {
        return Ok(());
    }
    if key.unit_type == MONSTER {
        w.units
            .get_mut(&key)
            .expect("checked above")
            .stats
            .insert(6, i32::from(life) << 8);
    } else if let Some(i) = w.roster_find(key.guid) {
        w.roster[i].life = u32::from(life) * 100 / 128;
    }
    Ok(())
}
