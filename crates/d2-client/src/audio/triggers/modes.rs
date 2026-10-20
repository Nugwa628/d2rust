//! Mode sounds (U, m, explicit): hit/death with impact layers,
//! attack swings and voices, block, kick, monster skill voices and the
//! dead-mode voice stop.

use d2_data::tables::Monsounds;

use super::{
    class_record, detach_groups, sid, swing_entry, Ctx, TriggerError, TriggerSound, Unit,
    UnitSound, MONSTER, PLAYER,
};
use crate::bridge::world::UnitKey;

/// Player mode codes used here.
pub mod pm {
    pub const DT: u8 = 0;
    pub const GH: u8 = 4;
    pub const A1: u8 = 7;
    pub const A2: u8 = 8;
    pub const BL: u8 = 9;
    pub const KK: u8 = 12;
    pub const S3: u8 = 15;
    pub const S4: u8 = 16;
    pub const DEAD: u8 = 17;
    pub const KB: u8 = 19;
}

/// Monster mode codes used here.
pub mod mm {
    pub const DT: u8 = 0;
    pub const GH: u8 = 3;
    pub const A1: u8 = 4;
    pub const A2: u8 = 5;
    pub const S1: u8 = 8;
    pub const S4: u8 = 11;
    pub const DD: u8 = 12;
    pub const KB: u8 = 13;
}

/// `diablo_death` and its two extra requests (id, delay).
pub const DIABLO_DEATH: i32 = 745;
pub const DIABLO_DEATH_EXTRA: [(i32, u32); 2] = [(746, 106), (747, 193)];
/// Attack voice gap (C).
pub const ATTACK_VOICE_GAP: u32 = 15;
/// `creature_chicken_1` and its gap (C).
pub const CHICKEN: i32 = 2692;
pub const CHICKEN_GAP: u32 = 25;

/// Mode dispatch (U, m, explicit). The caller passes
/// m = U's current mode for mode sets (explicit = 0) and the explicit m
/// otherwise; objects and other types do nothing here.
pub fn mode_sound(cx: &mut Ctx, u: &Unit, us: &mut UnitSound, m: u8) -> Result<(), TriggerError> {
    match u.identity() {
        PLAYER => match m {
            pm::DT | pm::GH | pm::KB => hit_death(cx, u, us, m)?,
            pm::A1 | pm::A2 => player_attack(cx, u, us)?,
            pm::BL => block(cx, u, us),
            pm::KK => {
                let d = if u.class == 6 { 4 } else { 0 };
                cx.req(255, Some(u.key), d);
            }
            pm::S3 if u.class == 4 => player_attack(cx, u, us)?,
            pm::S4 if u.class == 6 => player_attack(cx, u, us)?,
            _ => {}
        },
        MONSTER => {
            let Some(r) = u.monsounds else {
                return Ok(());
            };
            match convert_mode(r, m) {
                m2 @ (mm::DT | mm::GH | mm::KB) => hit_death(cx, u, us, m2)?,
                mm::A1 => monster_attack(cx, u, us, r, 1),
                mm::A2 => monster_attack(cx, u, us, r, 2),
                m2 @ mm::S1..=mm::S4 => skill_voice(cx, u, us, r, m2),
                mm::DD => monster_dead(cx.s, u.key, r),
                _ => {}
            }
        }
        _ => {}
    }
    Ok(())
}

/// One client mode set: the mode sounds of the new mode
/// (`m` = U's current mode for a mode set, explicit = 0; the explicit
/// callers pass their own m) come first, then the skill start sounds of
/// the same mode set when the mode set belongs to a skill start.
pub fn mode_set(
    cx: &mut Ctx,
    u: &Unit,
    us: &mut UnitSound,
    m: u8,
    skill: Option<(&super::skills::SkillStart, bool)>,
) -> Result<(), TriggerError> {
    mode_sound(cx, u, us, m)?;
    if let Some((sk, start_ok)) = skill {
        super::skills::skill_start(cx, u, sk, start_ok)?;
    }
    Ok(())
}

/// Monster mode conversion: for i = 1, 2, 3 in order, m = CvtMo_i
/// and CvtSk_i < 0 → m := CvtTgt_i.
pub fn convert_mode(r: &Monsounds, mut m: u8) -> u8 {
    for (mo, sk, tgt) in [
        (r.cvtmo1, r.cvtsk1, r.cvttgt1),
        (r.cvtmo2, r.cvtsk2, r.cvttgt2),
        (r.cvtmo3, r.cvtsk3, r.cvttgt3),
    ] {
        if m == mo && (sk as i32) < 0 {
            m = tgt;
        }
    }
    m
}

/// Impact sounds (a, b) of (U, h, m).
pub fn impact(h: u8, unit_type: u8, critter: bool, m: u8) -> (i32, i32) {
    if unit_type == MONSTER && critter {
        return (0, 0);
    }
    let a = match h & 0xF {
        1 => 321,
        2..=5 => 341,
        6 | 7 => 335,
        8 | 9 => 329,
        10 | 11 => 347,
        12 => 325,
        _ => 0,
    };
    let other = unit_type != PLAYER && unit_type != MONSTER;
    let b = match h & 0xF0 {
        0x10 if a != 0 => 379,
        0x20 => {
            if a != 0 {
                356
            } else {
                353
            }
        }
        0x30 => {
            if a == 0 {
                359
            } else if other || m != 0 {
                362
            } else {
                0
            }
        }
        0x40 => {
            if a != 0 {
                372
            } else {
                368
            }
        }
        0x50 if a == 0 || other || m != 0 => 376,
        0x60 => 382,
        0x70 => 395,
        0x80 => 2390,
        0xB0 => 391,
        _ => 0,
    };
    (a, b)
}

/// Hit and death (U, m).
pub fn hit_death(cx: &mut Ctx, u: &Unit, us: &mut UnitSound, m: u8) -> Result<(), TriggerError> {
    let monster = u.identity() == MONSTER;
    // Step 1.
    if monster && u.state_146 {
        return Ok(());
    }
    // Steps 2, 3.
    let h = us.hit_class;
    let (a, b) = impact(h, u.identity(), u.critter, m);
    let ha = cx.req(a, Some(u.key), 0);
    cx.req(b, Some(u.key), 0);
    if ha != 0 && u.is_local {
        cx.s.set_volume(ha, 255);
    }
    // Step 4. The sample-lock release (U, −1) is cache only
    // and has no call here.
    if monster && m == 0 {
        if let Some(r) = u.monsounds {
            monster_dead(cx.s, u.key, r);
        }
    }
    // Step 5.
    if u.frozen || h & 0xF0 == 0xA0 {
        return Ok(());
    }
    if monster {
        // Step 6.
        if let Some(r) = u.monsounds {
            let (id, d) = if m == mm::GH || m == mm::KB {
                if cx.s.speaking(u.key) {
                    (0, 0)
                } else {
                    (sid(r.hitsound), r.hitdelay)
                }
            } else {
                (sid(r.deathsound), r.deadelay)
            };
            if id != 0 {
                cx.req(id, Some(u.key), d);
                if id == DIABLO_DEATH {
                    for (x, dx) in DIABLO_DEATH_EXTRA {
                        cx.req(x, Some(u.key), dx);
                    }
                }
            }
        }
    } else if u.identity() == PLAYER {
        // Step 7.
        let rec = class_record(u.class)?;
        if m == pm::GH || m == pm::KB {
            if !cx.s.speaking(u.key) {
                cx.req(rec.hit, Some(u.key), 2);
            }
        } else {
            cx.req(rec.death, Some(u.key), 1);
        }
    }
    // Step 8.
    cx.voiced(us);
    Ok(())
}

/// Player attack (U, m, 1).
pub fn player_attack(cx: &mut Ctx, u: &Unit, us: &mut UnitSound) -> Result<(), TriggerError> {
    player_attack_with(cx, u, us, true)
}

/// The player attack (U, m, with_delay): d = swing(U, h), or 0 when the
/// third argument is 0 (animation event 3 passes 0).
pub fn player_attack_with(
    cx: &mut Ctx,
    u: &Unit,
    us: &mut UnitSound,
    with_delay: bool,
) -> Result<(), TriggerError> {
    let h = u.weapon_hit_class;
    let (id, frames) = swing_entry(h)?;
    if id != 0 {
        let d = if with_delay {
            super::delay_ticks(frames, u.speed)
        } else {
            0
        };
        cx.req(id, Some(u.key), d);
    }
    us.last_voice = cx.c;
    Ok(())
}

/// Monster attack, slot `n` = 1 (A1) or 2 (A2).
pub fn monster_attack(cx: &mut Ctx, u: &Unit, us: &mut UnitSound, r: &Monsounds, n: u8) {
    let (attack, del, prb, weapon, wdel, wvol) = if n == 1 {
        (
            r.attack1, r.att1del, r.att1prb, r.weapon1, r.wea1del, r.wea1vol,
        )
    } else {
        (
            r.attack2, r.att2del, r.att2prb, r.weapon2, r.wea2del, r.wea2vol,
        )
    };
    let prb = prb as i32;
    // Step 2.1.
    let mut ok = attack != 0;
    let mut skip = false;
    if prb < 100 {
        if cx.c.wrapping_sub(cx.g.last_voice_any)
            < cx.s.tick_rate().ticks_for_ms(ATTACK_VOICE_GAP * 40)
        {
            ok = false;
        }
        if cx.s.speaking(u.key) {
            skip = true;
        }
    }
    // Step 2.2 (Prb ≥ 100 still draws).
    if ok && !skip && (cx.s.roll(100) as i32) < prb {
        detach_neutral(cx.s, u.key, sid(r.neutral));
        cx.req(sid(attack), Some(u.key), del);
        cx.voiced(us);
    }
    // Step 2.3.
    let d = super::delay_ticks(wdel, u.speed);
    let h = cx.req(sid(weapon), Some(u.key), d);
    if h != 0 {
        cx.s.set_volume(h, wvol as i32);
    }
}

/// Detach the neutral voice (U, 1): each request of U in `Neutral`'s group
/// is detached with force when it has other units, else faded to 0 over 6.
fn detach_neutral(s: &mut dyn TriggerSound, unit: UnitKey, neutral: i32) {
    let base = s.group_base(neutral);
    for (h, id) in s.unit_requests(unit) {
        if s.group_base(id) == base {
            if s.unit_count(h) > 1 {
                s.detach(h, unit, true);
            } else {
                s.fade(h, 0, 0, 6);
            }
        }
    }
}

/// Block, player m = 9.
pub fn block(cx: &mut Ctx, u: &Unit, us: &mut UnitSound) {
    let id = match us.hit_class & 0xF {
        1..=9 => 398,
        10 | 11 => 401,
        _ => 0,
    };
    cx.req(id, Some(u.key), 0);
    cx.voiced(us);
}

/// Monster skill voice (U, m), m = 8..11.
pub fn skill_voice(cx: &mut Ctx, u: &Unit, us: &mut UnitSound, r: &Monsounds, m: u8) {
    let id = match m {
        8 => r.skill1,
        9 => r.skill2,
        10 => r.skill3,
        11 => r.skill4,
        _ => 0,
    } as i32;
    if id == 0
        || (id == CHICKEN
            && cx.c.wrapping_sub(us.last_voice) < cx.s.tick_rate().ticks_for_ms(CHICKEN_GAP * 40))
    {
        return;
    }
    cx.req(id, Some(u.key), 0);
    cx.voiced(us);
}

/// Monster dead, m = 12: detach with force every
/// request of U in the groups of its voices.
pub fn monster_dead(s: &mut dyn TriggerSound, unit: UnitKey, r: &Monsounds) {
    let ids = [
        r.attack1, r.attack2, r.hitsound, r.neutral, r.skill1, r.skill2, r.skill3, r.skill4,
        r.init, r.taunt, r.flee,
    ]
    .map(sid);
    detach_groups(s, unit, &ids);
}
