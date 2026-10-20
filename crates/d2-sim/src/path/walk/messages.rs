//! Byte builders of the S→C messages walking touches, and the choice of
//! message in the update pass. Who sends them and when is
//! the update pass's; 0x96's sender is the
//! client vitals sync (owned by combat vitals;
//! `combat::vitals::sync`), which builds it with [`walk_verify`].

use crate::path::record::DynamicPath;

/// S→C 0x0D PlayerStop (13 bytes): type, GUID, a, x, y, b, life %.
pub fn player_stop(ty: u8, guid: u32, a: u8, x: u16, y: u16, b: u8, life_pct: u8) -> [u8; 13] {
    let mut m = [0u8; 13];
    m[0] = 0x0D;
    m[1] = ty;
    m[2..6].copy_from_slice(&guid.to_le_bytes());
    m[6] = a;
    m[7..9].copy_from_slice(&x.to_le_bytes());
    m[9..11].copy_from_slice(&y.to_le_bytes());
    m[11] = b;
    m[12] = life_pct;
    m
}

/// S→C 0x0F PlayerMove (16 bytes): type, GUID, code, target x, y, 0, x, y.
pub fn player_move(
    ty: u8,
    guid: u32,
    code: u8,
    target_x: u16,
    target_y: u16,
    x: u16,
    y: u16,
) -> [u8; 16] {
    let mut m = [0u8; 16];
    m[0] = 0x0F;
    m[1] = ty;
    m[2..6].copy_from_slice(&guid.to_le_bytes());
    m[6] = code;
    m[7..9].copy_from_slice(&target_x.to_le_bytes());
    m[9..11].copy_from_slice(&target_y.to_le_bytes());
    m[11] = 0;
    m[12..14].copy_from_slice(&x.to_le_bytes());
    m[14..16].copy_from_slice(&y.to_le_bytes());
    m
}

/// S→C 0x10 PlayerToTarget (16 bytes): type, GUID, code, target type,
/// target GUID, x, y.
pub fn player_to_target(
    ty: u8,
    guid: u32,
    code: u8,
    target_ty: u8,
    target_guid: u32,
    x: u16,
    y: u16,
) -> [u8; 16] {
    let mut m = [0u8; 16];
    m[0] = 0x10;
    m[1] = ty;
    m[2..6].copy_from_slice(&guid.to_le_bytes());
    m[6] = code;
    m[7] = target_ty;
    m[8..12].copy_from_slice(&target_guid.to_le_bytes());
    m[12..14].copy_from_slice(&x.to_le_bytes());
    m[14..16].copy_from_slice(&y.to_le_bytes());
    m
}

/// S→C 0x15 ReassignPlayer (11 bytes): type, GUID, x, y, flag.
pub fn reassign_player(ty: u8, guid: u32, x: u16, y: u16, flag: u8) -> [u8; 11] {
    let mut m = [0u8; 11];
    m[0] = 0x15;
    m[1] = ty;
    m[2..6].copy_from_slice(&guid.to_le_bytes());
    m[6..8].copy_from_slice(&x.to_le_bytes());
    m[8..10].copy_from_slice(&y.to_le_bytes());
    m[10] = flag;
    m
}

/// Writes `value`'s low `width` bits LSB-first at bit `at` (the `bits:`
/// rule of the client-message layouts (`layout` column): fields `name:width` from bit 0
/// of byte 0).
fn put_bits(buf: &mut [u8], at: usize, width: usize, value: u32) {
    for i in 0..width {
        if (value >> i) & 1 != 0 {
            let b = at + i;
            buf[b / 8] |= 1 << (b % 8);
        }
    }
}

/// S→C 0x96 WalkVerify (9 bytes, bit-packed `id:8 stamina:15 x:16 y:16
/// dx:8 dy:8`). Each value is cut to its field width.
pub fn walk_verify(stamina: u32, x: u16, y: u16, dx: i8, dy: i8) -> [u8; 9] {
    let mut m = [0u8; 9];
    let mut at = 0;
    for (width, value) in [
        (8, 0x96u32),
        (15, stamina),
        (16, x as u32),
        (16, y as u32),
        (8, dx as u8 as u32),
        (8, dy as u8 as u32),
    ] {
        put_bits(&mut m, at, width, value);
        at += width;
    }
    m
}

/// Codes of the walk modes' update rows (table): (code to
/// point, code to unit). Walk and town walk share the row.
pub fn mode_codes(mode: u32) -> Option<(u8, u8)> {
    match mode {
        2 | 6 => Some((1, 0)),
        3 => Some((0x17, 0x18)),
        _ => None,
    }
}

/// The walk modes' update function for one client (rule
/// 2): nothing for the client whose player it is; else 0x10 with a target
/// unit, 0x0F without. `target` is T of rule 2, (type, GUID), already
/// filtered by the caller. `None` for other
/// modes (their own rows) or the own client.
pub fn mode_update(
    mode: u32,
    ty: u8,
    guid: u32,
    path: &DynamicPath,
    target: Option<(u8, u32)>,
    own_client: bool,
) -> Option<[u8; 16]> {
    let (to_point, to_unit) = mode_codes(mode)?;
    if own_client {
        return None;
    }
    let pos = path.cell();
    Some(match target {
        Some((t_ty, t_guid)) => {
            player_to_target(ty, guid, to_unit, t_ty, t_guid, pos.x as u16, pos.y as u16)
        }
        None => player_move(
            ty,
            guid,
            to_point,
            path.target_x,
            path.target_y,
            pos.x as u16,
            pos.y as u16,
        ),
    })
}

/// Flags-2 bits of rule 3.
pub const FLAGS2_RESYNC: u32 = 0x10000;
pub const FLAGS2_RESYNC_OTHERS: u32 = 0x800;

/// Whether the update pass sends 0x15 to a client, and its flag byte
/// bit 0x10000 → flag 1; bit 0x800 when the
/// client's player is not this unit → flag 0.
pub fn reassign_flag(flags2: u32, own_client: bool) -> Option<u8> {
    if flags2 & FLAGS2_RESYNC != 0 {
        Some(1)
    } else if flags2 & FLAGS2_RESYNC_OTHERS != 0 && !own_client {
        Some(0)
    } else {
        None
    }
}
