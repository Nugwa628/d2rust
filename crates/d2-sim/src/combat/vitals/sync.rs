//! The client vitals sync: what a player's own client is told about its life, mana,
//! stamina, position, gold and experience at the end of each flush.
//!
//! The messages are built from a per-client cache of what was last sent, with the layouts
//! S→C 0x18, 0x95 and 0x96; gold (0x19–0x1F) goes through
//! `items::moves::layouts::gold`, experience is 0x1A–0x1C.
//!
//! Pure: the caller reads the current values ([`Current`], with [`life_prediction`] /
//! [`mana_prediction`] for the potion predictions) and owns the cache; [`sync`] decides
//! and builds the bytes.

use crate::items::moves::layouts;
use crate::path::walk::messages::walk_verify;

/// The per-client cache record: what the client was last sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncCache {
    /// Syncs in a row that sent no life, mana or walk-verify message.
    pub quiet: u32,
    /// Life as last sent.
    pub life: u16,
    /// Mana as last sent.
    pub mana: u16,
    /// Stamina as last sent.
    pub stamina: u16,
    /// Life potion prediction as last sent.
    pub lp: u8,
    pub mp: u8,
    /// Sub-tile x as last sent.
    pub x: u16,
    pub y: u16,
    /// Path delta x as last sent (zero-extended byte).
    pub dx: u16,
    /// Path delta y as last sent (zero-extended byte).
    pub dy: u16,
    /// Gold sent (stat 14).
    pub gold: u32,
    /// Experience sent (stat 13).
    pub exp: u32,
}

/// The unit's current values that the sync reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Current {
    /// Life: total of stat 6 >> 8.
    pub life: i32,
    /// Max life >> 8.
    pub max_life: i32,
    /// Mana: total of stat 8 >> 8.
    pub mana: i32,
    /// Stamina: total of stat 10 >> 8.
    pub stamina: i32,
    /// Life potion prediction (percent byte, see `life_prediction`).
    pub lp: u8,
    pub mp: u8,
    /// Sub-tile x position.
    pub x: u16,
    pub y: u16,
    /// (X − path target) & 0xFF; 0 without a path.
    pub dx: u8,
    /// (Y − path target) & 0xFF; 0 without a path.
    pub dy: u8,
    /// Gold: total of stat 14.
    pub gold: u32,
    /// Experience: total of stat 13.
    pub exp: u32,
}

/// Whether the sync is forced: the client's update counter is at least 20, or at least 10
/// with a queued buffer.
pub fn force(update_count: u32, queued: bool) -> bool {
    update_count >= 20 || (update_count >= 10 && queued)
}

/// The low byte of v, 100 when that byte is above 100.
fn pct_byte(v: i32) -> u8 {
    let b = v as u8;
    if b > 100 {
        100
    } else {
        b
    }
}

/// Life potion prediction (percent of max life, 0–100). `potion`: the state 100
/// (`healthpot`) list as (its total of stat 74, its expire frame), when the unit has one.
/// 32-bit arithmetic.
pub fn life_prediction(potion: Option<(i32, i32)>, frame: i32, life_total: i32, m: i32) -> u8 {
    let Some((regen, expire)) = potion else {
        return 0;
    };
    if m == 0 {
        return 0;
    }
    let q = regen
        .wrapping_mul(expire.wrapping_sub(frame))
        .wrapping_add(life_total)
        >> 8;
    pct_byte(q.wrapping_mul(100).wrapping_div(m))
}

/// The inputs of the mana prediction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ManaInputs {
    /// Max mana (8.8 fixed point).
    pub max_mana: i32,
    /// charstats `ManaRegen` of the class row; `None`: no valid row.
    pub mana_regen: Option<u8>,
    /// Total of stat 27, the mana regeneration percent.
    pub regen_pct: i32,
    /// Total of stat 26, the flat mana regeneration added.
    pub regen_add: i32,
    /// Total of stat 8, the current mana.
    pub mana_total: i32,
}

/// Mana potion prediction (percent of max mana, 0–100). `potion_expire`: the expire frame
/// of the state 106 (`manapot`) list, when the unit has one.
pub fn mana_prediction(potion_expire: Option<i32>, frame: i32, v: &ManaInputs) -> u8 {
    let (Some(expire), Some(regen)) = (potion_expire, v.mana_regen) else {
        return 0;
    };
    let m = v.max_mana;
    if m == 0 {
        return 0;
    }
    let q = if regen == 0 {
        7500
    } else {
        i32::from(regen) * 25
    };
    let i = (m / q).max(1);
    // The stat 27 scaling is truncated here (not `MulDiv`).
    let i = i
        .wrapping_mul(v.regen_pct.wrapping_add(100))
        .wrapping_div(100)
        .wrapping_add(v.regen_add);
    let v = expire
        .wrapping_sub(frame)
        .wrapping_mul(i)
        .wrapping_add(v.mana_total)
        .wrapping_mul(100)
        .wrapping_div(m);
    pct_byte(v)
}

/// Writes `value`'s low `width` bits LSB first at bit `at`.
fn put_bits(buf: &mut [u8], at: &mut usize, width: usize, value: u32) {
    for i in 0..width {
        if (value >> i) & 1 != 0 {
            let b = *at + i;
            buf[b / 8] |= 1 << (b % 8);
        }
    }
    *at += width;
}

/// Builds S→C 0x18 LifeManaUpdate: 15 bytes, 115 bits.
#[allow(clippy::too_many_arguments)]
pub fn life_mana_update(
    life: i32,
    mana: i32,
    stamina: i32,
    lp: u8,
    mp: u8,
    x: u16,
    y: u16,
    dx: u8,
    dy: u8,
) -> [u8; 15] {
    let mut m = [0u8; 15];
    let mut at = 0;
    for (w, v) in [
        (8, 0x18u32),
        (15, life as u32),
        (15, mana as u32),
        (15, stamina as u32),
        (7, u32::from(lp)),
        (7, u32::from(mp)),
        (16, u32::from(x)),
        (16, u32::from(y)),
        (8, u32::from(dx)),
        (8, u32::from(dy)),
    ] {
        put_bits(&mut m, &mut at, w, v);
    }
    m
}

/// Builds S→C 0x95 LifeManaUpdate2: 13 bytes, 101 bits.
#[allow(clippy::too_many_arguments)]
pub fn life_mana_update2(
    life: i32,
    mana: i32,
    stamina: i32,
    x: u16,
    y: u16,
    dx: u8,
    dy: u8,
) -> [u8; 13] {
    let mut m = [0u8; 13];
    let mut at = 0;
    for (w, v) in [
        (8, 0x95u32),
        (15, life as u32),
        (15, mana as u32),
        (15, stamina as u32),
        (16, u32::from(x)),
        (16, u32::from(y)),
        (8, u32::from(dx)),
        (8, u32::from(dy)),
    ] {
        put_bits(&mut m, &mut at, w, v);
    }
    m
}

/// The experience message for a change from `old` to `new` (0x1A, 0x1B or 0x1C by the
/// size of the change): `None` when equal.
pub fn exp_message(new: u32, old: u32) -> Option<Vec<u8>> {
    if new == old {
        return None;
    }
    let d = new.wrapping_sub(old);
    Some(if d > 0xFFFE {
        let mut b = vec![0x1C];
        b.extend_from_slice(&new.to_le_bytes());
        b
    } else if d >= 0xFF {
        let mut b = vec![0x1B];
        b.extend_from_slice(&(d as u16).to_le_bytes());
        b
    } else {
        vec![0x1A, d as u8]
    })
}

/// What one run of the routine did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Synced {
    /// The routine finished: the caller resets the client's update counter to 0.
    pub done: bool,
    /// The messages in send order.
    pub messages: Vec<Vec<u8>>,
}

/// Runs one sync of the cache against the current values, returning the messages to send.
pub fn sync(cache: &mut SyncCache, cur: &Current, force: bool) -> Synced {
    let mut out = Synced::default();
    // A missing client or unit is the caller's concern; a unit without max life sends nothing.
    if cur.max_life <= 0 {
        return out;
    }
    // Unforced, skip a small life change (under 10 percent of max), or a drop to zero.
    if !force {
        let d = (i32::from(cache.life) - cur.life).wrapping_abs();
        if d.wrapping_mul(100) / cur.max_life < 10 {
            return out;
        }
        if cur.life == 0 && cache.life != 0 {
            return out;
        }
    }
    // Pick the message: potion predictions changed (0x18), life or mana changed (0x95),
    // or stamina changed / the position drifted while quiet (a walk verify). The cache
    // fields hold the values as stored (u16, u8).
    let (life, mana, stamina) = (cur.life as u16, cur.mana as u16, cur.stamina as u16);
    let msg: Option<Vec<u8>> = if cur.lp != cache.lp || cur.mp != cache.mp {
        Some(
            life_mana_update(
                cur.life,
                cur.mana,
                cur.stamina,
                cur.lp,
                cur.mp,
                cur.x,
                cur.y,
                cur.dx,
                cur.dy,
            )
            .to_vec(),
        )
    } else if life != cache.life || mana != cache.mana {
        Some(
            life_mana_update2(
                cur.life,
                cur.mana,
                cur.stamina,
                cur.x,
                cur.y,
                cur.dx,
                cur.dy,
            )
            .to_vec(),
        )
    } else if stamina != cache.stamina
        || (cache.quiet > 3
            && ((i32::from(cache.x) - i32::from(cur.x)).abs() >= 2
                || (i32::from(cache.y) - i32::from(cur.y)).abs() >= 2))
    {
        Some(walk_verify(cur.stamina as u32, cur.x, cur.y, cur.dx as i8, cur.dy as i8).to_vec())
    } else {
        None
    };
    finish(cache, cur, msg, &mut out);
    out
}

/// The forced sync when a player joins: the unit check as in [`sync`], then S→C 0x95
/// (the cache-dependent 0x18 and 0x96 are not taken at a join), then the gold,
/// experience and cache updates.
pub fn join(cache: &mut SyncCache, cur: &Current) -> Synced {
    let mut out = Synced::default();
    if cur.max_life <= 0 {
        return out;
    }
    let m = life_mana_update2(
        cur.life,
        cur.mana,
        cur.stamina,
        cur.x,
        cur.y,
        cur.dx,
        cur.dy,
    )
    .to_vec();
    finish(cache, cur, Some(m), &mut out);
    out
}

/// Applies the cache writes for `msg`, then sends gold and experience changes and stores
/// the current values.
fn finish(cache: &mut SyncCache, cur: &Current, msg: Option<Vec<u8>>, out: &mut Synced) {
    let (life, mana, stamina) = (cur.life as u16, cur.mana as u16, cur.stamina as u16);
    match msg {
        Some(m) => {
            out.messages.push(m);
            cache.x = cur.x;
            cache.y = cur.y;
            cache.dx = u16::from(cur.dx);
            cache.dy = u16::from(cur.dy);
            cache.quiet = 0;
        }
        None => cache.quiet = cache.quiet.wrapping_add(1),
    }
    // Gold.
    if cur.gold != cache.gold {
        if let Some(m) = layouts::gold(cur.gold, cache.gold) {
            out.messages.push(m);
        }
        cache.gold = cur.gold;
    }
    // Experience.
    if let Some(m) = exp_message(cur.exp, cache.exp) {
        out.messages.push(m);
    }
    cache.exp = cur.exp;
    // Remember what was sent.
    cache.life = life;
    cache.mana = mana;
    cache.stamina = stamina;
    cache.lp = cur.lp;
    cache.mp = cur.mp;
    out.done = true;
}

#[cfg(test)]
mod tests;
