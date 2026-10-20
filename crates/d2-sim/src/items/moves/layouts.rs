//! S→C byte layouts of the item messages and the gold messages. Little-endian;
//! the machine copy is `sim/server-messages.tsv` (checked in the tests).

use super::{Guid, MoveFatal, MAX_ITEM_MSG};

/// 0x9C: [1] action, [2] total size, [3] category, [4..7] item GUID,
/// [8..] item bit stream.
pub fn item_world(action: u8, category: u8, item: Guid, bits: &[u8]) -> Result<Vec<u8>, MoveFatal> {
    let size = 8 + bits.len();
    if size >= MAX_ITEM_MSG {
        return Err(MoveFatal::MessageSize(size));
    }
    let mut b = vec![0x9C, action, size as u8, category];
    b.extend_from_slice(&item.to_le_bytes());
    b.extend_from_slice(bits);
    Ok(b)
}

/// 0x9D: [1] action, [2] total size, [3] category, [4..7] item GUID,
/// [8] owner type, [9..12] owner GUID, [13..] item bit stream.
pub fn item_owned(
    action: u8,
    category: u8,
    item: Guid,
    owner_type: u8,
    owner: Guid,
    bits: &[u8],
) -> Result<Vec<u8>, MoveFatal> {
    let size = 13 + bits.len();
    if size >= MAX_ITEM_MSG {
        return Err(MoveFatal::MessageSize(size));
    }
    let mut b = vec![0x9D, action, size as u8, category];
    b.extend_from_slice(&item.to_le_bytes());
    b.push(owner_type);
    b.extend_from_slice(&owner.to_le_bytes());
    b.extend_from_slice(bits);
    Ok(b)
}

/// 0x7D (18 bytes): [1] owner type, [2..5] owner GUID, [6..9] item GUID,
/// [10..13] flag, [14..17] state.
pub fn item_state(owner_type: u8, owner: Guid, item: Guid, flag: u32, state: u32) -> Vec<u8> {
    let mut b = vec![0x7D, owner_type];
    b.extend_from_slice(&owner.to_le_bytes());
    b.extend_from_slice(&item.to_le_bytes());
    b.extend_from_slice(&flag.to_le_bytes());
    b.extend_from_slice(&state.to_le_bytes());
    b
}

fn relator(id: u8, unit_type: u8, arg: u8, unit: Guid) -> Vec<u8> {
    let mut b = vec![id, unit_type, arg];
    b.extend_from_slice(&unit.to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b
}

/// 0x47 (11 bytes): [1] unit type, [2] 0, [3..6] GUID, [7..10] 0.
pub fn relator1(unit_type: u8, unit: Guid) -> Vec<u8> {
    relator(0x47, unit_type, 0, unit)
}

/// 0x48 (11 bytes): [1] unit type, [2] argument, [3..6] GUID, [7..10] 0.
pub fn relator2(unit_type: u8, arg: u8, unit: Guid) -> Vec<u8> {
    relator(0x48, unit_type, arg, unit)
}

/// 0x42 (6 bytes): [1] unit type, [2..5] GUID.
pub fn clear_cursor(unit_type: u8, unit: Guid) -> Vec<u8> {
    let mut b = vec![0x42, unit_type];
    b.extend_from_slice(&unit.to_le_bytes());
    b
}

/// 0x3F (8 bytes): [1] code, [2..5] item GUID, [6..7] u16 argument.
pub fn use_stackable(code: u8, item: Guid, arg: u16) -> Vec<u8> {
    let mut b = vec![0x3F, code];
    b.extend_from_slice(&item.to_le_bytes());
    b.extend_from_slice(&arg.to_le_bytes());
    b
}

/// Gold sync: `None` when unchanged.
pub fn gold(new: u32, old: u32) -> Option<Vec<u8>> {
    if new == old {
        return None;
    }
    let delta = i64::from(new) - i64::from(old);
    Some(if (1..=254).contains(&delta) {
        vec![0x19, delta as u8]
    } else if new < 0xFF {
        vec![0x1D, 0x0E, new as u8]
    } else if new < 0xFFFF {
        let mut b = vec![0x1E, 0x0E];
        b.extend_from_slice(&(new as u16).to_le_bytes());
        b
    } else {
        let mut b = vec![0x1F, 0x0E];
        b.extend_from_slice(&new.to_le_bytes());
        b
    })
}

/// "Can't do that": S→C 0x5A, 40 bytes
/// `5A 0E 01` then 37 zero bytes.
pub fn cant_do_that() -> Vec<u8> {
    let mut b = vec![0u8; 40];
    b[0] = 0x5A;
    b[1] = 0x0E;
    b[2] = 0x01;
    b
}

/// S→C 0x7C (6 bytes): [1] unit type, [2..5] GUID.
pub fn item_used(unit_type: u8, unit: Guid) -> Vec<u8> {
    let mut b = vec![0x7C, unit_type];
    b.extend_from_slice(&unit.to_le_bytes());
    b
}
