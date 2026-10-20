//! The monster mode message with (game, unit, client) and
//! its nine builders: which of S→C 0x67–0x6D a monster's
//! mode produces, with which fields, as bytes. Pure: the caller (the
//! monster unit update, step 2, on the action
//! wiring) reads the unit, its path and its target into [`ModeInput`],
//! and applies [`ModeMessage::Stop`]'s stat 328 increment itself.

/// One row of the mode table (24 bytes per mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeRow {
    /// Keep the unit's target.
    pub use_target: bool,
    /// Without a target, (a, b) := the path target, else the unit's cell.
    pub target_from_path: bool,
    /// The moving builders (0x67 / 0x68).
    pub moving: bool,
    /// The attack builders (0x6B / 0x6C).
    pub attack: bool,
    /// The code sent without a target.
    pub code_to_point: u8,
    /// The code sent with a kept target.
    pub code_to_unit: u8,
}

const fn row(u: u8, p: u8, m: u8, a: u8, point: u8, unit: u8) -> ModeRow {
    ModeRow {
        use_target: u != 0,
        target_from_path: p != 0,
        moving: m != 0,
        attack: a != 0,
        code_to_point: point,
        code_to_unit: unit,
    }
}

/// The mode table, by monster mode.
pub const MODE_ROWS: [ModeRow; 16] = [
    row(0, 1, 0, 0, 8, 8),
    row(0, 0, 0, 0, 7, 7),
    row(1, 1, 1, 0, 1, 0),
    row(0, 0, 0, 0, 6, 6),
    row(1, 1, 0, 1, 11, 10),
    row(1, 1, 0, 1, 17, 16),
    row(1, 1, 0, 0, 18, 18),
    row(1, 1, 0, 1, 4, 5),
    row(1, 1, 0, 1, 12, 13),
    row(1, 1, 0, 1, 14, 15),
    row(1, 1, 0, 1, 26, 27),
    row(1, 1, 0, 1, 28, 29),
    row(0, 0, 0, 0, 9, 9),
    row(0, 1, 1, 0, 20, 20),
    row(1, 1, 0, 1, 12, 13),
    row(1, 1, 1, 0, 23, 24),
];

/// Monster modes the message names.
pub mod mode {
    pub const DT: u32 = 0;
    pub const NU: u32 = 1;
    pub const WL: u32 = 2;
    pub const GH: u32 = 3;
    pub const BL: u32 = 6;
    pub const DD: u32 = 12;
    pub const KB: u32 = 13;
    pub const SQ: u32 = 14;
    pub const RN: u32 = 15;
}

/// What reads from the unit, its path
/// and its target. The caller has already applied step 2 (the
/// refreshed target, dropped for a mode without "use
/// target" or out of the client's rooms) and checked that the unit has
/// a path (step 4's fatal 0xE6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModeInput {
    /// Unit +0x10.
    pub mode: u32,
    /// Unit +0x0C.
    pub guid: u32,
    /// The skill in use is not none.
    pub skill_in_use: bool,
    /// The kept target T (step 2): (type, GUID).
    pub target: Option<(u8, u32)>,
    /// The unit's cell (also,
    /// For a monster).
    pub cell: (u16, u16),
    /// The path target (+0x10, +0x12:).
    pub path_target: (u16, u16),
    /// The path direction +0x64.
    pub direction: u8,
    /// The path type +0x3C.
    pub path_type: u32,
    /// Path +0x90.
    pub path_90: u8,
    /// Path +0x91, the max distance.
    pub max_distance: u8,
    /// Path +0x93, the stop distance (adds 1).
    pub stop_distance: u8,
    /// Unit +0xB0.
    pub unit_b0: u8,
    /// The life fraction with (unit) (0..=0x80).
    pub life: u8,
    /// (mode 3's d byte).
    pub flag_100: bool,
    /// Stat 67 `velocitypercent` total.
    pub velocity: i32,
}

/// The outcome of the mode message build for one unit and client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModeMessage {
    /// Step 3: the skill message (0x4C with the kept target,
    /// else 0x4D), built by the caller from the used skill
    /// (`wiring::action::unit_update::skill_message`).
    Skill { to_unit: bool },
    /// Step 3: mode 14 with no skill in use: nothing.
    Nothing,
    /// Step 5, mode 1: S→C 0x6D; the caller then adds 1 to stat 328.
    Stop([u8; 10]),
    /// Every other mode: one of 0x67–0x6C.
    Send(Vec<u8>),
}

/// After its reads.
pub fn mode_message(i: &ModeInput) -> ModeMessage {
    if i.mode == mode::SQ || i.skill_in_use {
        if i.mode == mode::SQ && !i.skill_in_use {
            return ModeMessage::Nothing;
        }
        return ModeMessage::Skill {
            to_unit: i.target.is_some(),
        };
    }
    let Some(e) = MODE_ROWS.get(i.mode as usize) else {
        return ModeMessage::Nothing;
    };
    if i.mode == mode::NU {
        return ModeMessage::Stop(monster_stop(i.guid, i.cell.0, i.cell.1, i.life));
    }
    let moving_mode = matches!(i.mode, mode::WL | mode::RN);
    let typed = matches!(i.path_type, 5 | 6);
    // Step 4: the id family and code; (a, b) as two u16 halves of the
    // point forms, or (type, GUID) of the unit forms.
    let to_unit = match i.target {
        Some(_) if moving_mode && typed => None,
        t => t,
    };
    let point = if e.target_from_path {
        i.path_target
    } else {
        i.cell
    };
    // Step 5: d, e (and the moving s).
    let (mut d, mut ev) = (0u8, 0u8);
    let mut point = point;
    match i.mode {
        mode::DT => (d, ev) = (i.direction, i.unit_b0),
        mode::DD => (d, ev) = (i.direction, 0),
        mode::GH => {
            let l = if i.life > 1 { i.life - 1 } else { i.life };
            d = l | if i.flag_100 { 0x80 } else { 0 };
            ev = i.unit_b0;
        }
        mode::BL if to_unit.is_none() => point = (0, 0xFFFF),
        _ => {}
    }
    let velocity = i.velocity.clamp(-32768, 32767) as i16 as u16;
    if i.mode == mode::KB {
        let (d, f, ev) = (i.path_90, i.unit_b0, i.life);
        return ModeMessage::Send(match to_unit {
            Some((ty, guid)) => knockback_to_unit(KnockbackToUnit {
                guid: i.guid,
                code: e.code_to_unit,
                x: i.cell.0,
                y: i.cell.1,
                a: ty,
                b: guid,
                d,
                f,
                t: i.path_type,
                velocity,
                e: ev,
            })
            .to_vec(),
            None => {
                let (x, y) = if typed { i.path_target } else { i.cell };
                knockback(Knockback {
                    guid: i.guid,
                    code: e.code_to_point,
                    x,
                    y,
                    d,
                    f,
                    t: i.path_type,
                    velocity,
                    e: ev,
                })
                .to_vec()
            }
        });
    }
    if e.moving {
        let s = i.stop_distance.wrapping_add(1);
        return ModeMessage::Send(match to_unit {
            Some((ty, guid)) => move_to_unit(MoveToUnit {
                guid: i.guid,
                code: e.code_to_unit,
                x: i.cell.0,
                y: i.cell.1,
                a: ty,
                b: guid,
                s,
                t: i.path_type,
                velocity,
                max_distance: i.max_distance,
            })
            .to_vec(),
            None => {
                // Step 4: without T, (a, b) := the path target when the
                // row takes it from the path (WL, RN), else the cell; a
                // dropped T of path type 5 / 6 takes the path target.
                let (x, y) = if typed { i.path_target } else { point };
                monster_move(Move {
                    guid: i.guid,
                    code: e.code_to_point,
                    x,
                    y,
                    s,
                    t: i.path_type,
                    velocity,
                    max_distance: i.max_distance,
                })
                .to_vec()
            }
        });
    }
    ModeMessage::Send(match (e.attack, to_unit) {
        (true, Some((ty, guid))) => {
            monster_attack(i.guid, e.code_to_unit, ty, guid, d, i.cell.0, i.cell.1).to_vec()
        }
        (true, None) => monster_action(
            i.guid,
            e.code_to_point,
            point.0,
            point.1,
            d,
            ev,
            i.cell.0,
            i.cell.1,
        )
        .to_vec(),
        (false, Some((ty, guid))) => state_to_unit(i.guid, e.code_to_unit, ty, guid, d).to_vec(),
        (false, None) => monster_state(i.guid, e.code_to_point, point.0, point.1, d, ev).to_vec(),
    })
}

// ---- the builders -------------------------------------------------------

fn head<const N: usize>(id: u8, guid: u32) -> [u8; N] {
    let mut m = [0u8; N];
    m[0] = id;
    m[1..5].copy_from_slice(&guid.to_le_bytes());
    m
}

fn put16(m: &mut [u8], at: usize, v: u16) {
    m[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put32(m: &mut [u8], at: usize, v: u32) {
    m[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// The path type byte of the 0x67 builders: 5 or 6 → 1,
/// 8 → 11, else the type (as a byte).
fn t_point(t: u32) -> u8 {
    match t {
        5 | 6 => 1,
        8 => 11,
        t => t as u8,
    }
}

/// Fields of 0x67 (modes 2, 15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Move {
    pub guid: u32,
    pub code: u8,
    /// (a, b) of the build's step 4: the path target (rows that take it from
    /// the path, or path type 5 / 6), else the unit's cell.
    pub x: u16,
    pub y: u16,
    pub s: u8,
    /// The path type before the rewrite.
    pub t: u32,
    pub velocity: u16,
    pub max_distance: u8,
}

/// S→C 0x67 MonsterMove (16 bytes): GUID, code, x, y,
/// s @10, t' @12, velocity @13, maxd @15.
pub fn monster_move(f: Move) -> [u8; 16] {
    let mut m = head::<16>(0x67, f.guid);
    m[5] = f.code;
    put16(&mut m, 6, f.x);
    put16(&mut m, 8, f.y);
    m[10] = f.s;
    m[12] = t_point(f.t);
    put16(&mut m, 13, f.velocity);
    m[15] = f.max_distance;
    m
}

/// Fields of 0x67 (mode 13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Knockback {
    pub guid: u32,
    pub code: u8,
    /// As [`Move::x`].
    pub x: u16,
    pub y: u16,
    /// Path +0x90.
    pub d: u8,
    /// Unit +0xB0.
    pub f: u8,
    pub t: u32,
    pub velocity: u16,
    /// The life fraction.
    pub e: u8,
}

/// S→C 0x67 (16 bytes): GUID, code, x, y, d @10,
/// f @11, t' @12 (as the mode message's t'), velocity @13, e @15.
pub fn knockback(f: Knockback) -> [u8; 16] {
    let mut m = head::<16>(0x67, f.guid);
    m[5] = f.code;
    put16(&mut m, 6, f.x);
    put16(&mut m, 8, f.y);
    m[10] = f.d;
    m[11] = f.f;
    m[12] = t_point(f.t);
    put16(&mut m, 13, f.velocity);
    m[15] = f.e;
    m
}

/// Fields of 0x68 (modes 2, 15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoveToUnit {
    pub guid: u32,
    pub code: u8,
    /// The unit's cell.
    pub x: u16,
    pub y: u16,
    /// Target type and GUID.
    pub a: u8,
    pub b: u32,
    pub s: u8,
    pub t: u32,
    pub velocity: u16,
    pub max_distance: u8,
}

/// S→C 0x68 MonsterMoveToTarget (21 bytes): GUID,
/// code, x, y, a @10, b u32 @11, s @15, t' @17 (5 or 6 → 2, 8 → 11),
/// velocity @18, maxd @20.
pub fn move_to_unit(f: MoveToUnit) -> [u8; 21] {
    let mut m = head::<21>(0x68, f.guid);
    m[5] = f.code;
    put16(&mut m, 6, f.x);
    put16(&mut m, 8, f.y);
    m[10] = f.a;
    put32(&mut m, 11, f.b);
    m[15] = f.s;
    m[17] = match f.t {
        5 | 6 => 2,
        8 => 11,
        t => t as u8,
    };
    put16(&mut m, 18, f.velocity);
    m[20] = f.max_distance;
    m
}

/// Fields of 0x68 (mode 13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnockbackToUnit {
    pub guid: u32,
    pub code: u8,
    pub x: u16,
    pub y: u16,
    pub a: u8,
    pub b: u32,
    pub d: u8,
    pub f: u8,
    pub t: u32,
    pub velocity: u16,
    pub e: u8,
}

/// S→C 0x68 (21 bytes): GUID, code, x, y, a @10,
/// b u32 @11, d @15, f @16, t' @17 (8 → 11 only), velocity @18, e @20.
pub fn knockback_to_unit(f: KnockbackToUnit) -> [u8; 21] {
    let mut m = head::<21>(0x68, f.guid);
    m[5] = f.code;
    put16(&mut m, 6, f.x);
    put16(&mut m, 8, f.y);
    m[10] = f.a;
    put32(&mut m, 11, f.b);
    m[15] = f.d;
    m[16] = f.f;
    m[17] = match f.t {
        8 => 11,
        t => t as u8,
    };
    put16(&mut m, 18, f.velocity);
    m[20] = f.e;
    m
}

/// S→C 0x69 MonsterState (12 bytes): GUID, code @5,
/// a u16 @6, b u16 @8, d @10, e @11.
pub fn monster_state(guid: u32, code: u8, a: u16, b: u16, d: u8, e: u8) -> [u8; 12] {
    let mut m = head::<12>(0x69, guid);
    m[5] = code;
    put16(&mut m, 6, a);
    put16(&mut m, 8, b);
    m[10] = d;
    m[11] = e;
    m
}

/// S→C 0x6A (12 bytes): GUID, code @5, target type
/// @6, target GUID u32 @7, d @11.
pub fn state_to_unit(guid: u32, code: u8, a: u8, b: u32, d: u8) -> [u8; 12] {
    let mut m = head::<12>(0x6A, guid);
    m[5] = code;
    m[6] = a;
    put32(&mut m, 7, b);
    m[11] = d;
    m
}

/// S→C 0x6B MonsterAction (16 bytes): GUID, code @5,
/// a u16 @6, b u16 @8, d @10, e @11, unit x @12, y @14.
#[allow(clippy::too_many_arguments)]
pub fn monster_action(
    guid: u32,
    code: u8,
    a: u16,
    b: u16,
    d: u8,
    e: u8,
    x: u16,
    y: u16,
) -> [u8; 16] {
    let mut m = head::<16>(0x6B, guid);
    m[5] = code;
    put16(&mut m, 6, a);
    put16(&mut m, 8, b);
    m[10] = d;
    m[11] = e;
    put16(&mut m, 12, x);
    put16(&mut m, 14, y);
    m
}

/// S→C 0x6C MonsterAttack (16 bytes): GUID, code @5,
/// target type @6, target GUID u32 @7, d @11, unit x @12, y @14.
pub fn monster_attack(guid: u32, code: u8, a: u8, b: u32, d: u8, x: u16, y: u16) -> [u8; 16] {
    let mut m = head::<16>(0x6C, guid);
    m[5] = code;
    m[6] = a;
    put32(&mut m, 7, b);
    m[11] = d;
    put16(&mut m, 12, x);
    put16(&mut m, 14, y);
    m
}

/// S→C 0x6D MonsterStop (10 bytes): GUID, x @5,
/// y @7, the life fraction @9.
pub fn monster_stop(guid: u32, x: u16, y: u16, life: u8) -> [u8; 10] {
    let mut m = head::<10>(0x6D, guid);
    put16(&mut m, 5, x);
    put16(&mut m, 7, y);
    m[9] = life;
    m
}

#[cfg(test)]
mod tests;
