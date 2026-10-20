//! Sound parts of the client umod hooks, the monster death branches, the
//! leap landing and the spider lay. The effects that
//! are not sounds (client missiles, overlays) are reported to the caller
//! in the order the game creates them, relative to the requests.

use super::{Ctx, Unit};

/// `necromancer_corpseexp_1`.
pub const CORPSE_EXPLOSION: i32 = 2458;
/// Client missiles of the umod hooks.
pub const MISSILE_CORPSE_EXPLODE: i32 = 117;
pub const MISSILE_FIRE_EXPLOSION: i32 = 82;
pub const MISSILE_PAIN_WORM: i32 = 545;
/// Umods that request sounds in phase 2.
pub const UMOD_FIRE: u8 = 9;
pub const UMOD_GOBOOM: u8 = 31;
pub const UMOD_WORMS_ON_DEATH: u8 = 40;
/// The phase whose entries request sounds.
pub const SOUND_PHASE: u8 = 2;

/// Entry of the umod hook table (5 dwords per umod):
/// `phase + 5 × umod`.
pub fn umod_hook_slot(umod: u8, phase: u8) -> usize {
    usize::from(phase) + 5 * usize::from(umod)
}

/// What a hook or branch creates besides sounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Client missile `id` at (x, y).
    Missile { id: i32, x: i32, y: i32 },
    /// Overlay `id` on the unit.
    Overlay(i32),
}

/// The five explosion points: (x, y), (x + 1, y + 1), (x + 1,
/// y − 1), (x − 1, y + 1), (x − 1, y − 1).
pub fn explosion_points(x: i32, y: i32) -> [(i32, i32); 5] {
    [
        (x, y),
        (x + 1, y + 1),
        (x + 1, y - 1),
        (x - 1, y + 1),
        (x - 1, y - 1),
    ]
}

/// U's frame (+0x44 >> 8, arithmetic) is 4, mode 0 (DT) and `unique` ≠ 0.
fn explosion_due(u: &Unit, unique: bool) -> bool {
    unique && u.mode == 0 && (u.frame as i32) >> 8 == 4
}

/// Phase-2 hook of umod `umod` on U at (x, y); `unique` is
/// the type flag 0x08. Effects are reported through `fx` in creation order
/// before the request. Runs every client update (a frame value of 4 that
/// lasts several updates repeats it).
pub fn umod_phase2(
    cx: &mut Ctx,
    u: &Unit,
    umod: u8,
    unique: bool,
    pos: (i32, i32),
    fx: &mut dyn FnMut(Effect),
) {
    match umod {
        UMOD_FIRE | UMOD_GOBOOM => {
            if !explosion_due(u, unique) {
                return;
            }
            let pts = explosion_points(pos.0, pos.1);
            if umod == UMOD_FIRE {
                for (x, y) in pts {
                    fx(Effect::Missile {
                        id: MISSILE_CORPSE_EXPLODE,
                        x,
                        y,
                    });
                }
            }
            for (x, y) in pts {
                fx(Effect::Missile {
                    id: MISSILE_FIRE_EXPLOSION,
                    x,
                    y,
                });
            }
            cx.req(CORPSE_EXPLOSION, Some(u.key), 0);
        }
        UMOD_WORMS_ON_DEATH => worms(cx, u, pos, fx),
        _ => {}
    }
}

/// Umod 40 phase 2 and the state 110 `pregnant` remove hook:
/// no test at all: missile 117, missile 545 `pain worm appear` at U's
/// position, then the request.
pub fn worms(cx: &mut Ctx, u: &Unit, pos: (i32, i32), fx: &mut dyn FnMut(Effect)) {
    fx(Effect::Missile {
        id: MISSILE_CORPSE_EXPLODE,
        x: pos.0,
        y: pos.1,
    });
    fx(Effect::Missile {
        id: MISSILE_PAIN_WORM,
        x: pos.0,
        y: pos.1,
    });
    cx.req(CORPSE_EXPLOSION, Some(u.key), 0);
}

/// The direction fold table: the 64 directions to 8, `((d +
/// 4) >> 3) & 7`.
pub fn fold_direction(d: u8) -> u8 {
    ((d.wrapping_add(4)) >> 3) & 7
}

/// BaseIds of the code-8 branch.
pub const BASE_MINION1: i32 = 453;
pub const BASE_SUICIDEMINION1: i32 = 461;
pub const BASE_VINE: [i32; 3] = [425, 426, 427];
/// `minion_death_a1` … `_d1`, indexed by d8 & 3.
pub const MINION_DEATHS: [i32; 4] = [1308, 1311, 1314, 1317];
pub const SUICIDE_OVERLAY: i32 = 204;
pub const FIREBALL_IMPACT: i32 = 2419;
pub const VINE_BEAST_DEATH_MISSILE: i32 = 470;
pub const DRUIDPOD_DEATH: i32 = 790;

/// Before the mode set (the death mode sound of the mode sounds
/// follows it): minion death voice; suicide minion overlay 204 then 2,419;
/// vine creatures their death missile at U's position. `dir` is U's
/// direction byte (path +0x64; 0 without a path).
pub fn monster_death_before(
    cx: &mut Ctx,
    u: &Unit,
    base_id: i32,
    dir: u8,
    pos: (i32, i32),
    fx: &mut dyn FnMut(Effect),
) {
    match base_id {
        BASE_MINION1 => {
            let s = MINION_DEATHS[usize::from(fold_direction(dir) & 3)];
            cx.req(s, Some(u.key), 0);
        }
        BASE_SUICIDEMINION1 => {
            fx(Effect::Overlay(SUICIDE_OVERLAY));
            cx.req(FIREBALL_IMPACT, Some(u.key), 0);
        }
        b if BASE_VINE.contains(&b) => fx(Effect::Missile {
            id: VINE_BEAST_DEATH_MISSILE,
            x: pos.0,
            y: pos.1,
        }),
        // Other BaseId cases create overlays only.
        _ => {}
    }
}

/// After the mode set: vine creatures request 790; true when the
/// common tail is skipped.
pub fn monster_death_after(cx: &mut Ctx, u: &Unit, base_id: i32) -> bool {
    if BASE_VINE.contains(&base_id) {
        cx.req(DRUIDPOD_DEATH, Some(u.key), 0);
        true
    } else {
        false
    }
}

/// Skill flag bit of the leap landing check.
pub const LEAP_FLAG: u32 = 0x100;
pub const LEAP_LAND: i32 = 2517;
pub const DUST_OVERLAY: i32 = 80;
/// BaseId 78 `sandleaper1`.
pub const BASE_SANDLEAPER: i32 = 78;

/// `cltdofunc` 43 / 44 run the landing check every client update while the
/// skill's flags have bit 0x100.
pub fn leap_check_runs(skill_flags: u32) -> bool {
    skill_flags & LEAP_FLAG != 0
}

/// Not landed when the leap height h is non-zero and U's
/// position differs from the skill's target point.
pub fn leap_landed(height: i32, at_target: bool) -> bool {
    height == 0 || at_target
}

/// The leap landing: overlay 80, request the running
/// footstep `footstep` (0 → nothing), then 2,517 for a non-monster; a
/// monster of BaseId 78 makes its own landing call (no request); any other monster
/// nothing. Returns whether it landed.
pub fn leap_landing(
    cx: &mut Ctx,
    u: &Unit,
    identity_monster: bool,
    height: i32,
    at_target: bool,
    footstep: i32,
    fx: &mut dyn FnMut(Effect),
) -> bool {
    if !leap_landed(height, at_target) {
        return false;
    }
    fx(Effect::Overlay(DUST_OVERLAY));
    if footstep > 0 {
        cx.req(footstep, Some(u.key), 0);
    }
    if !identity_monster {
        cx.req(LEAP_LAND, Some(u.key), 0);
    }
    true
}

/// `spider_web_1` and the offsets of the spider lay.
pub const SPIDER_WEB: i32 = 1830;
pub const SPIDER_MISSILE: i32 = 143;
/// Skill 173 `SpiderLay`.
pub const SPIDER_LAY_SKILL: i32 = 173;
/// State 22 `spiderlay`.
pub const STATE_SPIDERLAY: u16 = 22;
const D16: [u8; 8] = [10, 8, 22, 20, 18, 16, 14, 12];

/// d16 = [10, 8, 22, 20, 18, 16, 14, 12][d8] with d8 the folded path
/// direction (+0x65); indexes the offset tables.
pub fn spider_offset_index(path_dir: u8) -> u8 {
    D16[usize::from(fold_direction(path_dir))]
}

/// The spider lay: acts when U has state 22; needs a path with flag
/// 0x08; the room at the offset point in town → nothing. Else the lay
/// missile 143 is made (`make_missile` returns whether it was created) and
/// 1,830 is requested whether or not it was.
pub fn spider_lay(
    cx: &mut Ctx,
    u: &Unit,
    has_state_spiderlay: bool,
    path_flag_08: bool,
    offset_room_in_town: bool,
    make_missile: &mut dyn FnMut() -> bool,
) {
    if !has_state_spiderlay || !path_flag_08 || offset_room_in_town {
        return;
    }
    make_missile();
    cx.req(SPIDER_WEB, Some(u.key), 0);
}
