//! The interact sender with (type, GUID) and its two callers: the
//! player mode request code 0x02 and the
//! local-player command 0x13 of `ClientFn` 13 (with the
//! action gate).

use super::super::dispatch::HandlerError;
use super::super::output::Output;
use super::super::world::{
    ClientWorld, ModeRequest, ModelInputs, UnitKey, ITEM, MONSTER, OBJECT, PLAYER, TILE,
};
use super::{ObjFx, ObjSound};

/// C→S 0x13 interact (9 bytes).
pub const INTERACT: u8 = 0x13;
/// C→S 0x16 item pick-up (13 bytes).
pub const PICKUP: u8 = 0x16;
/// The monster interact repeat window, ms.
pub const MONSTER_REPEAT_MS: u32 = 200;
/// The mode request code that sends the interact.
pub const CODE_INTERACT: u8 = 0x02;
/// The player mode that ignores a flag-0 request.
pub const MODE_SEQUENCE: u32 = 0x13;

/// C→S 0x13 {0x13, type u32, GUID u32}.
pub fn interact_bytes(unit_type: u32, guid: u32) -> Vec<u8> {
    let mut m = vec![INTERACT];
    m.extend_from_slice(&unit_type.to_le_bytes());
    m.extend_from_slice(&guid.to_le_bytes());
    m
}

/// The interact sender with (type, GUID). U is looked
/// up in set S; none → nothing. Sends append to `outgoing`; the sound
/// and effect calls are returned in call order.
/// TODO: the path reset of
/// the object case has no client path record in the model.
pub fn send(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    unit_type: u16,
    guid: u32,
) -> Result<Vec<Output>, HandlerError> {
    let mut out = Vec::new();
    let Ok(t) = u8::try_from(unit_type) else {
        return Ok(out);
    };
    let key = UnitKey::new(t, guid);
    if !w.units.contains_key(&key) {
        return Ok(out);
    }
    let p = w.local_player.filter(|k| w.units.contains_key(k));
    let ty = u32::from(unit_type);
    match t {
        PLAYER => {
            face(w, p, key)?;
            w.outgoing.push(interact_bytes(ty, guid));
        }
        MONSTER => {
            let now = inputs.now;
            let u = w.units.get_mut(&key).expect("looked up above");
            if now.wrapping_sub(u.interact_ms) >= MONSTER_REPEAT_MS {
                u.interact_ms = now;
                w.outgoing.push(interact_bytes(ty, guid));
            }
        }
        OBJECT => object(w, inputs, p, key, &mut out)?,
        ITEM => {
            let mut m = vec![PICKUP];
            m.extend_from_slice(&4u32.to_le_bytes());
            m.extend_from_slice(&guid.to_le_bytes());
            m.extend_from_slice(&u32::from(inputs.objclient.pickup_flag).to_le_bytes());
            w.outgoing.push(m);
        }
        // PROVISIONAL: sends 0x13 (T, g)
        // for a tile, lists no tile case: a tile (a
        // level warp) is sent as the 9-byte
        // 0x13 of the other types. The base-game server takes that form
        // (13, u32 5, u32 GUID); left open: the form the base-game
        // client's own click on a tile sends.
        TILE => w.outgoing.push(interact_bytes(ty, guid)),
        _ => {}
    }
    Ok(out)
}

/// P faces U (with U's client point): kept as the
/// turn's input (`ClientUnit::turned_toward`).
fn face(w: &mut ClientWorld, p: Option<UnitKey>, key: UnitKey) -> Result<(), HandlerError> {
    let p = p.ok_or(HandlerError::Crash {
        at: 0x0048_0930,
        what: "the local player (none) in the interact sender",
    })?;
    if let Some(pu) = w.units.get_mut(&p) {
        pu.turned_toward = Some(key);
    }
    Ok(())
}

/// The object case of step 7.
fn object(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    p: Option<UnitKey>,
    key: UnitKey,
    out: &mut Vec<Output>,
) -> Result<(), HandlerError> {
    let (flag_4, class, mode) = {
        let u = &w.units[&key];
        (u.flag_4, u.class, u.mode)
    };
    if !flag_4 {
        w.outgoing.push(interact_bytes(u32::from(OBJECT), key.guid));
        return Ok(());
    }
    face(w, p, key)?;
    let p = p.expect("face checked it");
    let mut r = [0i32; 7];
    r[0] = side_skill_id(w, p);
    let needs = match (class, mode) {
        (404, 0) => Some(*b"qf2 "),
        (376, 2) => Some(*b"hfh "),
        _ => None,
    };
    if let Some(code) = needs {
        if inputs.objclient.hand_code != Some(code) {
            out.push(Output::ObjectSound(ObjSound::PlayerEvent {
                player: p,
                event: INTERACT,
            }));
            return Ok(());
        }
        // The id of P's native entry of skill 0:
        // 0, and 0 too when P has none (of a null entry).
        r[0] = 0;
    }
    if !(class == 376 && mode == 0) {
        r[1] = -1;
        r[2] = 2;
        r[3] = key.guid as i32;
        out.push(Output::ObjectFx(ObjFx::SkillStart {
            player: p,
            record: r,
        }));
    }
    w.outgoing.push(interact_bytes(u32::from(OBJECT), key.guid));
    Ok(())
}

/// The id of P's highest entry of
/// skill 1 (`highest_entry`: entries of skill 1
/// with has-charges 0). The id is the skill (1), or 0 when P has no such
/// entry (reads 0 for a null entry).
fn side_skill_id(w: &ClientWorld, p: UnitKey) -> i32 {
    let has = w.units[&p]
        .skills
        .as_ref()
        .is_some_and(|l| l.entries.iter().any(|e| e.skill == 1 && !e.has_charges));
    i32::from(has)
}

/// The player mode request code 0x02 of a queued message (flag 1),
/// masked to 16 bits.
pub fn mode_request_code_2(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    record: [i32; 7],
) -> Result<Vec<Output>, HandlerError> {
    send(w, inputs, (record[0] & 0xFFFF) as u16, record[1] as u32)
}

/// The action gate (P): 1 when P's current skill
/// is none, its id is out of range, its `skills` row has
/// the gate bit, or P's mode is 1 or 5; else 0.
pub fn action_gate(w: &ClientWorld, inputs: &ModelInputs, p: UnitKey) -> bool {
    let Some(pu) = w.units.get(&p) else {
        return true;
    };
    if matches!(pu.mode, 1 | 5) {
        return true;
    }
    let Some(e) = pu
        .skills
        .as_ref()
        .and_then(|l| l.current.and_then(|i| l.entries.get(i)))
    else {
        return true;
    };
    let id = usize::from(e.skill);
    id >= inputs.tables.skills.len()
        || inputs
            .objclient
            .skill_gate
            .get(id)
            .copied()
            .unwrap_or(false)
}

/// The local-player command 0x13 on (2, GUID):
/// the gate returns 0 → nothing; else P's mode request code 2 with
/// record {2, GUID, 0, …} and flag 0: stored like every
/// request; P in mode 0x13 ignores it; else the interact
/// sender. sends nothing more for command 0x13.
pub fn command_13(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    p: UnitKey,
    guid: u32,
) -> Result<Vec<Output>, HandlerError> {
    if !action_gate(w, inputs, p) {
        return Ok(Vec::new());
    }
    let record = [i32::from(OBJECT), guid as i32, 0, 0, 0, 0, 0];
    let Some(pu) = w.units.get_mut(&p) else {
        return Ok(Vec::new());
    };
    pu.last_mode_request = Some(ModeRequest {
        code: CODE_INTERACT,
        record,
    });
    pu.last_path_request = Some((
        ModeRequest {
            code: CODE_INTERACT,
            record,
        },
        pu.mode_requests,
    ));
    if pu.mode == MODE_SEQUENCE {
        return Ok(Vec::new());
    }
    mode_request_code_2(w, inputs, record)
}
