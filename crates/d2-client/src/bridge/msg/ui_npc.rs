//! NPC messages: 0x28 NPC dialog start and quest flags, 0x62 NPC dialog
//! end, 0x2A NPC transaction, 0x4E / 0x4F hire list, 0x8A NPC wants to
//! interact, 0x91 NPC intros, 0x9B hireling revive state. Each does its
//! model part at receive and emits one output for the UI layer.

use super::super::dispatch::{HandlerError, Message};
use super::super::output::{NpcDialog, Output};
use super::super::world::{
    ClientWorld, KindData, UnitKey, DIALOG_REPLY_SLOT, MONSTER, OBJECT, PLAYER,
};
use super::Bytes;

fn len(msg: &Message<'_>, n: usize, what: &'static str) -> Result<(), HandlerError> {
    if msg.bytes.len() == n {
        Ok(())
    } else {
        Err(HandlerError::Invalid(what))
    }
}

fn le(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

/// 0x28 QuestInfo: type T u8@1, GUID G u32@2, R u8@6, quest flags
/// Q (96 bytes) @7.
pub fn quest_info(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 103, "0x28 is 103 bytes")?;
    let b = Bytes(msg.bytes);
    let (t, g, r) = (b.u8(1)?, b.u32(2)?, b.u8(6)?);
    let mut q = [0u8; 96];
    q.copy_from_slice(b.slice(7, 96)?);
    // Step 2.
    if t == 6 {
        w.quest_player = Some(q);
        msg.out.push(Output::QuestFlags { record: q });
        return Ok(());
    }
    let key = UnitKey::new(t, g);
    if !w.units.contains_key(&key) {
        // Step 3: the local player's player data +0x150 … +0x15C := 0
        // (no model field: no rule writes them otherwise, as 0x04);
        // C→S 0x30 with u32 R, u32 G.
        let mut m = vec![0x30];
        m.extend(le(u32::from(r)));
        m.extend(le(g));
        w.outgoing.push(m);
        msg.out.push(Output::NpcGone { guid: g });
        return Ok(());
    }
    // Step 4, the model part: unit flag 0x2 := 1, C→S 0x2F (u32 T, u32 G).
    let u = w.units.get_mut(&key).expect("checked above");
    u.flag_2 = Some(true);
    u.quest_untargetable = false;
    let class = u.class;
    let mut m = vec![0x2F];
    m.extend(le(u32::from(t)));
    m.extend(le(g));
    w.outgoing.push(m);
    // Open question 10, decided as A: the dialog branch's C→S 0x31 keeps
    // its place right after 0x2F; the bridge fills the slot when the UI
    // hands back its case ([`apply_dialog_branch`]).
    w.outgoing.push(DIALOG_REPLY_SLOT);
    // The captured inputs of the UI branch.
    let monsters = &msg.inputs.tables.monsters;
    let row = |c: u32| monsters.get(c as usize).copied().flatten();
    let cursor_item = matches!(
        w.local().map(|u| &u.kind),
        Some(KindData::Player(p)) if p.cursor_item.is_some()
    );
    let npc_monsters = w
        .units
        .values()
        .filter(|u| u.key.unit_type == MONSTER && row(u.class).is_some_and(|r| r.npc))
        .map(|u| u.key)
        .collect();
    // The dialog branch decides on UI state: the UI layer
    // evaluates it and hands the case back; its model writes and C→S
    // 0x31 are the bridge's ([`apply_dialog_branch`], open question 10
    // decided as A: stands).
    msg.out.push(Output::NpcDialog(Box::new(NpcDialog {
        kind: t,
        guid: g,
        quest_flags: q,
        unit: key,
        class,
        interact: row(class).is_some_and(|r| r.interact),
        f4b1a10: Some(crate::bridge::output::f4b1a10(class)),
        cursor_item,
        level: w.local().map_or(1, |u| w.base(u.key, 12, 0)),
        unidentified: crate::ui::original::npc_box::unidentified_count(w),
        expansion: w.expansion != 0,
        npc_monsters,
    })));
    Ok(())
}

/// The case of 0x28's dialog branch the UI layer chose (the
/// first that holds). B3–B6 write the model alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogCase {
    /// ≠ 0.
    B0,
    /// The local player has a cursor item.
    B1,
    /// m = the text's id ≠ 0xFFFF: C→S 0x31 with m.
    B2 { m: u32 },
    /// B3, B4, B5 or B6.
    Rest,
}

/// Classes whose `interact` NPC does not take mode 1 and face the player.
const NO_FACE: [u32; 4] = [537, 538, 539, 527];

/// The model writes and the C→S 0x31 of 0x28's dialog branch (the bold
/// effects), in base-game order, for the case the UI layer chose:
///
/// 1. B0: unit flag 0x2 := 0; nothing else.
/// 2. Otherwise: an `interact` class other than 537, 538, 539, 527 → U's
///    mode := 1 and U turns toward the local player.
/// 3. B1: nothing more. B2: C→S 0x31 (`31`, u32 G, u32 m) in the slot
///    reserved after 0x2F.
/// 4. B3–B6: class ≠ 527 and the local player in mode 1 or 5 → the
///    local player turns toward U; U's path stops.
///
/// The slot is cleared in every case but B2. A unit no longer present
/// takes no model write (a later message of the frame removed it).
pub fn apply_dialog_branch(w: &mut ClientWorld, d: &NpcDialog, case: DialogCase) {
    let mut reply = None;
    let local = w.local_player;
    w.interact_npc = Some(d.unit);
    if case == DialogCase::B0 {
        if let Some(u) = w.units.get_mut(&d.unit) {
            u.flag_2 = Some(false);
        }
    } else {
        if d.interact && !NO_FACE.contains(&d.class) {
            if let Some(u) = w.units.get_mut(&d.unit) {
                u.mode = 1;
                u.turned_toward = local;
            }
            // on U's path toward the local player's client
            // point: the new direction and turn step, which
            // the monster update turns by (Akara 0, 4, 8, 12, 15 over
            // frames 15-19).
            let at = local.and_then(|k| w.units.get(&k)).map(|l| {
                let (cx, cy) = w.local_predicted.map_or_else(
                    || {
                        l.precise
                            .map_or_else(|| l.cell(), |(x, y)| ((x >> 16) as u16, (y >> 16) as u16))
                    },
                    |(x, y)| ((x >> 16) as u16, (y >> 16) as u16),
                );
                (cx, cy)
            });
            if let Some((x, y)) = at {
                super::super::critter_path::face_point(w, d.unit, x, y);
                super::super::critter_path::turn_now(w, d.unit);
            }
        }
        match case {
            DialogCase::B2 { m } => {
                let mut b = vec![0x31];
                b.extend(le(d.guid));
                b.extend(le(m));
                reply = Some(b);
            }
            DialogCase::Rest => {
                if d.class != 527 {
                    if let Some(l) = local.and_then(|k| w.units.get_mut(&k)) {
                        if l.key.unit_type == PLAYER && matches!(l.mode, 1 | 5) {
                            l.turned_toward = Some(d.unit);
                        }
                    }
                }
                if let Some(u) = w.units.get_mut(&d.unit) {
                    u.path_stopped = true;
                }
            }
            _ => {}
        }
    }
    if let Some(i) = w.outgoing.iter().position(|m| m.is_empty()) {
        match reply {
            Some(b) => w.outgoing[i] = b,
            None => {
                w.outgoing.remove(i);
            }
        }
    }
}

/// 0x62 MakeUnitTargetable: type T u8@1, GUID u32@2.
pub fn dialog_end(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 7, "0x62 is 7 bytes")?;
    let b = Bytes(msg.bytes);
    let t = b.u8(1)?;
    match t {
        1 => {
            if let Some(u) = w.units.get_mut(&UnitKey::new(MONSTER, b.u32(2)?)) {
                u.flag_2 = Some(true);
                u.quest_untargetable = false;
            }
        }
        2 | 4 | 6 => {}
        _ => return Ok(()),
    }
    msg.out.push(Output::NpcDialogEnd { kind: t });
    Ok(())
}

/// 0x2A NpcTransaction: the 15 bytes and the player's gold after the
/// transaction (the message's u32 at 11, written from stat 14). The local
/// player's gold stat takes that value: a fee or purchase is not followed
/// by a separate stat message.
pub fn npc_transaction(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 15, "0x2A is 15 bytes")?;
    let mut bytes = [0u8; 15];
    bytes.copy_from_slice(msg.bytes);
    let gold = i32::from_le_bytes([bytes[11], bytes[12], bytes[13], bytes[14]]);
    if let Some(key) = w.local_player {
        if let Some(unit) = w.units.get_mut(&key) {
            unit.stats.insert(14, gold);
        }
    }
    msg.out.push(Output::NpcTransaction { bytes, gold });
    Ok(())
}

/// 0x4E MercForHire: name u16@1, seed u32@3.
pub fn hire_offer(_: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 7, "0x4E is 7 bytes")?;
    let b = Bytes(msg.bytes);
    msg.out.push(Output::HireOffer {
        name: b.u16(1)?,
        seed: b.u32(3)?,
    });
    Ok(())
}

/// 0x4F StartMercList.
pub fn hire_list_reset(_: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 1, "0x4F is 1 byte")?;
    msg.out.push(Output::HireListReset);
    Ok(())
}

/// `eunuch harem blocker` (`objects.txt` 318).
pub const HAREM_BLOCKER: u32 = 318;

/// 0x8A NpcWantsInteract: type u8@1, GUID u32@2.
pub fn npc_interact(w: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 6, "0x8A is 6 bytes")?;
    let b = Bytes(msg.bytes);
    let key = UnitKey::new(b.u8(1)?, b.u32(2)?);
    let unit = w.units.get(&key);
    let blocker_open = w
        .units
        .values()
        .any(|u| u.key.unit_type == OBJECT && u.class == HAREM_BLOCKER && u.mode == 2);
    msg.out.push(Output::NpcInteract {
        unit: key,
        present: unit.is_some(),
        class: unit.map_or(0, |u| u.class),
        // Monster data +0x3C: its only writer is the 0xAC create, with
        // the bit-stream `value` (−1 when it was not sent).
        mdata_3c: unit.and_then(|u| match &u.kind {
            KindData::Monster(m) => Some(m.value),
            _ if u.key.unit_type == MONSTER => Some(-1),
            _ => None,
        }),
        blocker_open,
    });
    Ok(())
}

/// 0x91 NpcGossipAct: u8@1 (not read), 12 class slots u16@2+2k.
pub fn npc_intro(_: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 26, "0x91 is 26 bytes")?;
    let b = Bytes(msg.bytes);
    let mut slots = [0u16; 12];
    for (k, s) in slots.iter_mut().enumerate() {
        *s = b.u16(2 + 2 * k)?;
    }
    msg.out.push(Output::NpcIntro { slots });
    Ok(())
}

/// 0x9B: u16@1, u16@3.
pub fn merc_revive(_: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    len(msg, 7, "0x9B is 7 bytes")?;
    let b = Bytes(msg.bytes);
    msg.out.push(Output::MercRevive {
        state: b.u16(1)?,
        value: b.u16(3)?,
    });
    Ok(())
}
