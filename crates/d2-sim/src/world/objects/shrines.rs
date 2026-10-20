// Spec: specs/world/objects.md §9
//! Shrines: operate 2 (§9.1), the effects (§9.2, §9.3), events 5 and 6.
//!
//! Stats, states, items, missiles, monsters and hovers belong to other
//! specs; they are reached through [`ShrineWorld`]. All math is 32-bit
//! integer math as the original's (wrapping, division toward zero); life,
//! mana and stamina are 8.8 fixed.

use crate::units::UnitId;

use super::{
    oevent, oflags, schedule_endanim, set_flag, set_mode, ObjectControl, ObjectError, ObjectHost,
    ObjectTables, ObjectWorld, Operate, TOWNS,
};

#[cfg(test)]
mod tests;

/// Stat ids read or written here (`sim/stats.md`; §9.2).
pub mod stat {
    pub const LIFE: u16 = 6;
    pub const MANA: u16 = 8;
    pub const STAMINA: u16 = 10;
    pub const MAX_STAMINA: u16 = 11;
    pub const TO_HIT: u16 = 19;
    pub const MIN_DAMAGE: u16 = 21;
    pub const ITEM_ARMOR_PERCENT: u16 = 25;
    pub const STAMINA_RECOVERY: u16 = 28;
    pub const MIN_DAMAGE_2H: u16 = 23;
    pub const STAMINA_BONUS: u16 = 162;
}

/// The hover string of shrine id `id`: `"%d"`(3683 + id) (§9.1 rule 3).
pub const HOVER_STRING_BASE: u32 = 3683;
/// Event 6 delay after the hover is created (§9.1 rule 3).
pub const HOVER_EVENT_DELAY: i32 = 300;
/// Frames per `reset time in minutes` (§9.1 rule 5).
pub const RESET_FRAMES_PER_MINUTE: i32 = 1200;
/// Effect table bound (`0x00732EAC`, §9.1 rule 4).
pub const EFFECT_BOUND: u8 = 24;
/// Free-spot mask and size of the portal shrine (code 17, §9.2).
pub const PORTAL_SPOT_MASK: u32 = 0x1C09;
pub const PORTAL_SPOT_SIZE: i32 = 3;
/// Town portal object class (code 17).
pub const TOWN_PORTAL_CLASS: u16 = 59;
/// Missile flags of the exploding and poison shrines (§9.3).
pub const POTION_MISSILE_FLAGS: u32 = 0x520;
/// Storm missiles: flags 3, position given and target relative (§9.3).
pub const STORM_MISSILE_FLAGS: u32 = 3;
/// The gem shrine's fallback chipped gems by `roll(6)` (§9.3).
pub const CHIPPED_GEMS: [[u8; 4]; 6] = [*b"gcw ", *b"gcr ", *b"gcg ", *b"gcb ", *b"gcy ", *b"gcv "];
/// The "no better gem" code (§9.3).
pub const NO_GEM: [u8; 4] = *b"non ";
/// Exploding / poison potion offsets of the 6 missiles (§9.3).
pub const POTION_OFFSETS: [(i32, i32); 6] = [(-6, 6), (-6, -6), (0, 6), (0, -6), (6, 6), (6, -6)];

/// Remove callback ids of the shrine states (§9.2).
pub const SKILL_REMOVE: u32 = 0x0058_3BD0;
pub const STAMINA_REMOVE: u32 = 0x0058_3A40;

/// The remove callback of a shrine state (§9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveCallback {
    /// None given: the helper's default `0x0056E900` (`skills/bodies.md`
    /// §2.8).
    Default,
    /// `0x00583BD0` (code 12, skill shrine).
    Skill,
    /// `0x00583A40` (code 14, stamina shrine).
    Stamina,
}

/// One timed-state request: `0x0056E970` through `0x00582800`
/// (`skills/bodies.md` §2.7) with the shrine's arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateRequest {
    /// P (the target).
    pub target: UnitId,
    /// The shrine object (the source).
    pub source: UnitId,
    /// `Duration in frames` (+0x0C).
    pub duration: i32,
    pub state: u16,
    pub stat: u16,
    pub value: i32,
    pub remove: RemoveCallback,
}

/// A stat list a state request returned (stat-lists spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StateList(pub u32);

/// One gem in the player's backpack (item type 20, §9.3), in walk order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gem {
    pub item: UnitId,
    /// The better-gem code (`non ` = none).
    pub better: [u8; 4],
}

/// One missile creation request (missiles spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissileRequest {
    pub class: u16,
    pub owner: UnitId,
    /// The unit the missile starts from (the shrine).
    pub from: UnitId,
    /// Target offset from `from`'s position.
    pub offset: (i32, i32),
    pub skill_level: i32,
    pub flags: u32,
}

/// The shrine seams beyond [`ObjectWorld`]. Every default is the narrowest
/// reading: nothing happens, or none / 0.
#[allow(unused_variables)]
pub trait ShrineWorld: ObjectWorld {
    /// `0x00661110` (hover spec): create the object's hover with string
    /// `string_id`; lifetime 8 · text length + 125 frames from the current
    /// frame (see [`hover_lifetime`]); store it at unit +0xA4. `false`: not
    /// created.
    fn create_hover(&mut self, obj: UnitId, string_id: u32) -> bool {
        false
    }
    /// The expiry frame of the object's hover (unit +0xA4); `None`: none.
    fn hover_expiry(&self, obj: UnitId) -> Option<i32> {
        None
    }
    /// Free the object's hover (none: nothing).
    fn free_hover(&mut self, obj: UnitId) {}

    /// The unit's stat `id` (unit getter `0x00625480`, `sim/stats.md`).
    fn stat(&self, unit: UnitId, id: u16) -> i32 {
        0
    }
    /// Set the unit's stat `id` (with its client update; `sim/stats.md`).
    fn set_stat(&mut self, unit: UnitId, id: u16, value: i32) {}
    /// Add to the unit's base stat `id` (`sim/stats.md`).
    fn add_base_stat(&mut self, unit: UnitId, id: u16, delta: i32) {}
    /// `0x00625D10`: maximum life (`sim/stats.md`).
    fn max_life(&self, unit: UnitId) -> i32 {
        0
    }
    /// `0x00625D60`: maximum mana (`sim/stats.md`).
    fn max_mana(&self, unit: UnitId) -> i32 {
        0
    }
    /// `0x00625DB0`: maximum stamina (`sim/stats.md`).
    fn max_stamina(&self, unit: UnitId) -> i32 {
        0
    }

    /// The timed-state helper `0x0056E970` through `0x00582800`
    /// (`skills/bodies.md` §2.7). `None`: no list.
    fn apply_state(&mut self, req: StateRequest) -> Option<StateList> {
        None
    }
    /// Set stat `id` on a state's list (`0x00625D00`-family writer,
    /// `sim/stat-lists.md`).
    fn set_list_stat(&mut self, list: StateList, id: u16, value: i32) {}
    /// `0x0056DE40`: refresh the unit's skills (`skills/bodies.md` §2.8).
    fn refresh_skills(&mut self, unit: UnitId) {}
    /// `0x00583740`: P's to-hit rating of skill 0 with its weapon (combat
    /// spec).
    fn to_hit(&mut self, player: UnitId) -> i32 {
        0
    }
    /// The player wields a two-handed weapon (items spec).
    fn two_handed(&self, player: UnitId) -> bool {
        false
    }

    /// P's backpack gems (item type 20) in inventory walk order with their
    /// better-gem codes (items / inventory spec).
    fn backpack_gems(&self, player: UnitId) -> Vec<Gem> {
        Vec::new()
    }
    /// Remove `item` from P's inventory and free it (inventory spec).
    fn remove_item(&mut self, player: UnitId, item: UnitId) {}
    /// `0x00582AC0`: one item of `code` dropped near P (items spec);
    /// false when no free spot was found (nothing made).
    fn drop_near_player(&mut self, player: UnitId, code: [u8; 4]) -> bool {
        false
    }
    /// A potion of `code` and `quantity` dropped near P (exploding and
    /// poison shrines; items spec).
    fn drop_potion_near_player(&mut self, player: UnitId, code: [u8; 4], quantity: i32) {}

    /// Storm (`0x00582DA0`): the players and monsters the unit finder
    /// `0x0065A950` / `0x0065AC70` returns around `center` with filter
    /// `0x00582710`, those not dead (`0x005541B0`), in the finder's order
    /// (objects.md §9.3).
    fn units_in_range(&mut self, center: UnitId, range: i32) -> Vec<UnitId> {
        Vec::new()
    }
    /// Create a missile (missiles spec).
    fn create_missile(&mut self, m: MissileRequest) {}
    /// P's character level (stat 12, `sim/stats.md`).
    fn player_level(&self, player: UnitId) -> i32 {
        0
    }

    /// `0x0064E7B0`: a free spot near the unit's position of `size` free of
    /// `mask` (`sim/path-placement.md` §7). `None`: none found.
    fn free_spot(&mut self, unit: UnitId, size: i32, mask: u32) -> Option<(i32, i32)> {
        None
    }
    /// `0x00582A00` (§9.2 code 16): reverse P's name (player data +0x00)
    /// in place (`_strrev`, then `strncpy` of 16 bytes; no message).
    fn reverse_player_name(&mut self, player: UnitId) {}
    /// `0x0056D130`: a portal object of `class` at (x, y) in P's room to
    /// level `dest`, owner P (`world/waypoints.md`, quest specs).
    fn create_portal(&mut self, player: UnitId, x: i32, y: i32, class: u16, dest: u32) {}
    /// `0x0065A800` with callback `0x00582750`: the nearest eligible
    /// monster to P becomes unique (monsters spec, open question 5).
    fn make_nearest_unique(&mut self, player: UnitId) {}
}

/// The hover lifetime of a text of `len` characters (§9.1 rule 3).
pub fn hover_lifetime(len: u32) -> i32 {
    (len as i32).wrapping_mul(8).wrapping_add(125)
}

fn queue_flag<W: ObjectWorld>(w: &mut W, obj: UnitId, f: u32) {
    w.queue_update(obj);
    set_flag(w, obj, f, true);
}

/// Operate 2 `0x00583C70` (§9.1).
pub fn operate<W: ObjectHost>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    op: &Operate,
) -> Result<i32, ObjectError> {
    let obj = op.object;
    let d = *ctl.get(obj)?;
    // Rule 1.
    if d.operator != 0 || w.mode(obj) != 0 {
        return Ok(0);
    }
    // §9.1 "No guards in 1.14d": the record is read unchecked at rule 4
    // (a null read, fatal); every live operate-2 row sets it in init 1.
    // Reported before any change.
    let id = d.shrine.ok_or(ObjectError::NoRow {
        table: "shrines",
        row: u32::from(d.interact),
    })?;
    let s = t.shrine(id)?.clone();
    let o = t.object(op.class)?;
    // Rule 2.
    ctl.get_mut(obj)?.operator = op.operator.map_or(0, |p| w.guid(p).wrapping_add(1));
    queue_flag(w, obj, oflags::CHANGED);
    set_mode(t, w, obj, op.class, 1, true)?;
    // Rule 3.
    w.free_hover(obj);
    if w.create_hover(obj, HOVER_STRING_BASE + u32::from(id)) {
        queue_flag(w, obj, oflags::HOVER_FREED);
        let at = w.frame() + w.tick_rate().ticks(HOVER_EVENT_DELAY);
        w.schedule(obj, oevent::HOVER, at);
    }
    // Rule 4.
    // §9.1 "No guards": the effect gets a missing operator unchecked (a
    // null read, fatal); unreachable with live data.
    let p = op.operator.ok_or(ObjectError::ShrineNoOperator)?;
    effect(t, w, obj, p, &s)?;
    // Rule 5.
    let m = i32::from(s.reset_time_in_minutes);
    if m != 0 {
        let at = w.frame() + w.tick_rate().ticks(RESET_FRAMES_PER_MINUTE * m) + 1;
        w.schedule(obj, oevent::SHRINE_RESET, at);
    }
    // Rule 6.
    if w.mode(obj) == 1 && o.cycleanim1 == 0 && o.mode2 != 0 {
        schedule_endanim(w, o, obj);
    }
    Ok(1)
}

/// The table stat / state pair of the generic state effect `0x00583B30`
/// (§9.2).
pub fn state_pair(code: u8) -> Option<(u16, u16)> {
    Some(match code {
        6 => (171, 128),
        8 => (39, 131),
        9 => (43, 132),
        10 => (41, 130),
        11 => (45, 133),
        13 => (27, 135),
        15 => (85, 137),
        _ => return None,
    })
}

/// `V(stat, a)` `0x00583840` (§9.2).
pub fn value<W: ShrineWorld>(w: &mut W, player: UnitId, id: u16, a: i32) -> i32 {
    match id {
        11 | 27 | 39 | 41 | 43 | 45 | 85 | 171 => a,
        stat::TO_HIT => a.wrapping_mul(w.to_hit(player)) / 100,
        stat::MIN_DAMAGE => {
            let s = if w.two_handed(player) {
                stat::MIN_DAMAGE_2H
            } else {
                stat::MIN_DAMAGE
            };
            a.wrapping_mul(w.stat(player, s)) / 100
        }
        other => a.wrapping_mul(w.stat(player, other)) / 100,
    }
}

#[allow(clippy::too_many_arguments)]
fn state<W: ShrineWorld>(
    w: &mut W,
    obj: UnitId,
    p: UnitId,
    s: &d2_data::tables::Shrines,
    state: u16,
    stat: u16,
    value: i32,
    remove: RemoveCallback,
) -> Option<StateList> {
    w.apply_state(StateRequest {
        target: p,
        source: obj,
        duration: w.tick_rate().ticks(s.duration_in_frames as i32),
        state,
        stat,
        value,
        remove,
    })
}

/// The skill level of the storm and potion missiles: P level / 5 clamped
/// to 1..8 (§9.3).
pub fn missile_level(player_level: i32) -> i32 {
    (player_level / 5).clamp(1, 8)
}

/// The base-stat add `0x006272B0`: a 0 add writes nothing (§9.2).
fn base_add<W: ShrineWorld>(w: &mut W, unit: UnitId, id: u16, delta: i32) {
    if delta != 0 {
        w.add_base_stat(unit, id, delta);
    }
}

/// The effect of `code` (§9.1 rule 4, table `0x006E1850`, §9.2, §9.3).
pub fn effect<W: ObjectHost>(
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    p: UnitId,
    s: &d2_data::tables::Shrines,
) -> Result<(), ObjectError> {
    let a0 = s.arg0 as i32;
    let a1 = s.arg1 as i32;
    let code = if s.code < EFFECT_BOUND { s.code } else { 1 };
    match code {
        // Refill `0x005828E0` (codes 0, 1, 23 and every code ≥ 24).
        0 | 1 | 23 => {
            let life = w.stat(p, stat::LIFE);
            let max = w.max_life(p);
            w.add_base_stat(p, stat::LIFE, max.wrapping_sub(life));
            let mana = w.stat(p, stat::MANA);
            let max = w.max_mana(p);
            w.add_base_stat(p, stat::MANA, max.wrapping_sub(mana));
        }
        // `0x00582860`: life to max (edge case 5), a base-stat add.
        2 => {
            let max = w.max_life(p);
            let life = w.stat(p, stat::LIFE);
            base_add(w, p, stat::LIFE, max.wrapping_sub(life));
        }
        // `0x005828A0`: mana to max (edge case 5), a base-stat add.
        3 => {
            let max = w.max_mana(p);
            let mana = w.stat(p, stat::MANA);
            base_add(w, p, stat::MANA, max.wrapping_sub(mana));
        }
        // `0x00582940`: base-stat adds (`0x006272B0`, §9.2).
        4 => {
            let life = w.stat(p, stat::LIFE);
            let d = a0.wrapping_mul(life >> 8) / 100;
            let add = (a1 as u32).wrapping_mul(d as u32).wrapping_mul(256) / 100;
            base_add(w, p, stat::LIFE, d.wrapping_mul(256).wrapping_neg());
            base_add(w, p, stat::MANA, add as i32);
        }
        // `0x005829A0`: base-stat adds (§9.2).
        5 => {
            let mana = w.stat(p, stat::MANA);
            let d = a0.wrapping_mul(mana) / 100;
            let add = (a1 as u32).wrapping_mul(d as u32) / 100;
            base_add(w, p, stat::MANA, d.wrapping_neg());
            base_add(w, p, stat::LIFE, add as i32);
        }
        // `0x00583B30`.
        6 | 8..=11 | 13 | 15 => {
            if let Some((st, sv)) = state_pair(code) {
                let v = value(w, p, st, a0);
                state(w, obj, p, s, sv, st, v, RemoveCallback::Default);
            }
        }
        // `0x005839B0`.
        7 => {
            let v = value(w, p, stat::TO_HIT, a0);
            if let Some(l) = state(
                w,
                obj,
                p,
                s,
                129,
                stat::ITEM_ARMOR_PERCENT,
                a1,
                RemoveCallback::Default,
            ) {
                w.set_list_stat(l, stat::TO_HIT, v);
            }
        }
        // `0x00583BF0`.
        12 => {
            state(
                w,
                obj,
                p,
                s,
                134,
                stat::MAX_STAMINA,
                0,
                RemoveCallback::Skill,
            );
            w.refresh_skills(p);
        }
        // `0x00583A70` (edge case 6).
        14 => {
            let max = w.max_stamina(p);
            w.set_stat(p, stat::STAMINA, max);
            let v = value(w, p, stat::STAMINA_BONUS, a0);
            if let Some(l) = state(
                w,
                obj,
                p,
                s,
                136,
                stat::STAMINA_BONUS,
                v,
                RemoveCallback::Stamina,
            ) {
                w.set_list_stat(l, stat::STAMINA, v.wrapping_mul(2));
                w.set_list_stat(l, stat::STAMINA_RECOVERY, 1000);
            }
        }
        // `0x00582A00`: P's name reversed in place (unreachable through
        // init).
        16 => w.reverse_player_name(p),
        // `0x00582A30`.
        17 => portal_shrine(t, w, p),
        18 => gem(w, p),
        19 => storm(w, obj, p, a0, a1),
        // `0x00583050`.
        20 => w.make_nearest_unique(p),
        21 | 22 => potions(w, obj, p, code, a0, a1),
        _ => {}
    }
    Ok(())
}

/// Sound event of a refused portal creation in town (`0x0056D130`).
const PORTAL_REFUSED_SOUND: u8 = 0x18;

/// Code 17 `0x00582A30`.
fn portal_shrine<W: ObjectHost>(t: &ObjectTables, w: &mut W, p: UnitId) {
    // §9.2 code 17: no free spot → nothing; the portal is created in P's
    // room. P's act town by P's level (`levels.Act`, §5.5).
    let Some((x, y)) = w.free_spot(p, PORTAL_SPOT_SIZE, PORTAL_SPOT_MASK) else {
        return;
    };
    // `0x0056D130` (Wine 1.14d, gen-shrine-17): P's room in a town
    // (`0x0061AB00`) and the class not one of the quest portals → no
    // portal; sound event 0x18 on P, targeted at P (`0x00553380`), result
    // 0.
    if w.level(p).is_some_and(crate::world::waypoints::is_town) {
        w.sound(p, PORTAL_REFUSED_SOUND, Some(p), false);
        return;
    }
    let Some(act) = w.level(p).and_then(|l| t.level(l)).map(|l| l.act) else {
        return;
    };
    let Some(&town) = TOWNS.get(act as usize) else {
        return;
    };
    w.create_portal(p, x + 5, y + 5, TOWN_PORTAL_CLASS, town);
}

/// Code 18 `0x00582C40` (§9.3).
fn gem<W: ShrineWorld>(w: &mut W, p: UnitId) {
    for g in w.backpack_gems(p) {
        // `0x00582AC0` makes the better gem first and removes the old one
        // only when it was made; a failed drop goes on to the next gem.
        if g.better != NO_GEM && w.drop_near_player(p, g.better) {
            w.remove_item(p, g.item);
            return;
        }
    }
    let r = w.unit_seed(p).map_or(0, |s| s.roll(6)) as usize;
    w.drop_near_player(p, CHIPPED_GEMS[r]);
}

/// Code 19 `0x00582DA0` (§9.3).
fn storm<W: ShrineWorld>(w: &mut W, obj: UnitId, p: UnitId, a0: i32, a1: i32) {
    // §9.3 "Storm, read in 1.14d": a base-stat add on stat 6, nothing
    // tests the result or kills (`objects-2.md` §24 rule 3).
    for u in w.units_in_range(obj, a1) {
        let life = w.stat(u, stat::LIFE);
        let loss = ((life >> 8).wrapping_mul(a0) / 100).wrapping_mul(256);
        base_add(w, u, stat::LIFE, loss.wrapping_neg());
    }
    let lvl = missile_level(w.player_level(p));
    // i outer, j inner; flags 3 (position given, target relative).
    for i in 1..=4 {
        for j in 1..=4 {
            let sx = if i % 2 == 1 { 5 * i } else { -5 * i };
            let sy = if j % 2 == 1 { 5 * j } else { -5 * j };
            w.create_missile(MissileRequest {
                class: 62,
                owner: p,
                from: obj,
                offset: (sx, sy),
                skill_level: lvl,
                flags: STORM_MISSILE_FLAGS,
            });
        }
    }
}

/// Codes 21 `0x005830E0` and 22 `0x00583410` (§9.3).
fn potions<W: ShrineWorld>(w: &mut W, obj: UnitId, p: UnitId, code: u8, a0: i32, a1: i32) {
    let (potion, missile) = if code == 21 {
        (*b"opm ", 45)
    } else {
        (*b"gpm ", 48)
    };
    let r = w.unit_seed(obj).map_or(0, |s| s.roll(a1.wrapping_sub(a0))) as i32;
    let n = a0.wrapping_add(r);
    for _ in 0..n {
        w.drop_potion_near_player(p, potion, 1);
    }
    let lvl = missile_level(w.player_level(p));
    for offset in POTION_OFFSETS {
        w.create_missile(MissileRequest {
            class: missile,
            owner: p,
            from: obj,
            offset,
            skill_level: lvl,
            flags: POTION_MISSILE_FLAGS,
        });
    }
}

/// Event 5 `0x005814D0` (§9.1).
pub fn reset_event<W: ObjectHost>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
) -> Result<(), ObjectError> {
    let class = ctl.get(obj)?.class;
    if t.object(class)?.subclass & 1 == 0 {
        return Ok(());
    }
    queue_flag(w, obj, oflags::CHANGED);
    set_mode(t, w, obj, class, 0, true)?;
    ctl.get_mut(obj)?.operator = 0;
    Ok(())
}

/// Event 6 `0x00581620` (§9.1).
pub fn hover_event<W: ObjectHost>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
) -> Result<(), ObjectError> {
    let _ = (ctl, t);
    // `objects-2.md` §24 rule 4: no object or no hover → nothing, no
    // reschedule.
    let Some(e) = w.hover_expiry(obj) else {
        return Ok(());
    };
    if e <= w.frame() {
        w.free_hover(obj);
        queue_flag(w, obj, oflags::HOVER_FREED);
    } else {
        w.schedule(obj, oevent::HOVER, e);
    }
    Ok(())
}
