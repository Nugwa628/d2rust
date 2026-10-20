//! The client mode machines: the mode request with (code, U,
//! record, 1) that ends every unit-handler message,
//! dispatched by U's type:
//!
//! - player: the code table's mode, `+0xB0`
//!   (last hit class) and position-check writes; the
//!   interact sender of code 0x02 stays in [`super::objects::interact`];
//! - object: code 3 is the object
//!   mode change (mode :=, the object light of the new
//!   mode, the mode sound call inside the change); code 0x15 the
//!   shrine use (in `msg::units`); other codes
//!   fatal 0x39C;
//! - item: code 2 → mode :=, flag 0x2 :=
//!   (≠ 0);
//! - monster: the mode column of the
//!   dispatch, see [`monster`];
//! - missile: nothing.
//!
//! The hit overlays ([`hit_overlays`]) run from the
//! mode sets and from code 0x13 of both machines.
//!
//! Every request is also kept as `last_mode_request`; a
//! player's or monster's cast (0x15 / 0x16) also as `last_cast`.
//!
//! Local walk prediction (an open question) is not a mode
//! machine rule: PROVISIONAL: the local
//! player is drawn at the last server-sent position, no prediction;
//! settled by a capture of the client state over a held walk.

use d2_sim::monsters::mode_message::MODE_ROWS;

use super::check::check;
use super::dispatch::HandlerError;
use super::msg::lighting::object_light;
use super::objects::{ObjSound, ObjUnit};
use super::output::{Output, Outputs};
use super::world::{
    CastRequest, CastTarget, ClientWorld, KindData, ModeRequest, ModelInputs, UnitKey, ITEM,
    MISSILE, MONSTER, OBJECT, PLAYER,
};

/// Player modes the table names (player modes).
pub mod player_mode {
    pub const DEATH: u32 = 0;
    pub const NEUTRAL: u32 = 1;
    pub const WALK: u32 = 2;
    pub const RUN: u32 = 3;
    pub const GET_HIT: u32 = 4;
    pub const TOWN_NEUTRAL: u32 = 5;
    pub const TOWN_WALK: u32 = 6;
    pub const THROW: u32 = 9;
    pub const BLOCK: u32 = 0xD;
    pub const DEAD: u32 = 0x11;
    pub const SEQUENCE: u32 = 0x13;
}

/// The mode code 0x12 sets: player mode 9, `BL` in `PlrMode`.
const MODE_BLOCK_0X12: u32 = 9;

/// Player mode 16 `S4` (`PlrMode`): the Assassin's Left Hand Swing.
const MODE_S4: u32 = 16;

/// Item types of the code 0x12 test: 51 `shld`, 45 `weap`.
const SHLD: i16 = 51;
const WEAP: i16 = 45;

/// on a worn item: its flag word (item data +0x18: the
/// stream's F and the bits the model writes) has neither 0x100 (broken)
/// nor 0x4000.
fn valid_item(w: &ClientWorld, v: &super::items::ItemView) -> bool {
    let model = match w.units.get(&v.key).map(|u| &u.kind) {
        Some(KindData::Item(d)) => d.flags,
        _ => 0,
    };
    (v.flags | model) & (0x100 | 0x4000) == 0
}

/// The valid item `key` wears at body location `loc`.
fn valid_at(w: &ClientWorld, key: UnitKey, loc: u8) -> Option<super::items::ItemView> {
    super::attack_weapon::worn_at(w, key, loc).filter(|v| valid_item(w, v))
}

/// "Of type `t`" (with equivalence); no item tables or an
/// ear: no.
fn of_type(w: &ClientWorld, v: &super::items::ItemView, t: i16) -> bool {
    match (&w.item_tables.0, v.code) {
        (Some(tables), Some(code)) => tables.item_is_type(code, t),
        _ => false,
    }
}

/// With (inventory, 0): the item at body location 4, else 5, is
/// valid and of type 51 `shld`.
fn holds_shield(w: &ClientWorld, key: UnitKey) -> bool {
    [4u8, 5]
        .into_iter()
        .any(|l| valid_at(w, key, l).is_some_and(|v| of_type(w, &v, SHLD)))
}

/// The COF weapon class with (U, inventory, mode −1, 1) is 0xD
/// `ht2`: the draw
/// identity (its mode kept) is a player, not in mode 0 or
/// 0x11, of class 6 (Assassin); the hand item (location 4's valid item
/// when its `component` is 5 or 6, else location 5's) has component 5
/// or 6; and both locations hold a valid item of type 45 `weap`. No
/// other branch gives `ht2`: no base-game `weapons` `wclass` /
/// `2handedwclass` (307 rows) and no `monstats2` `BaseW` (610 rows) is
/// `ht2`.
fn holds_two_claws(w: &ClientWorld, inputs: &ModelInputs, key: UnitKey) -> bool {
    let Some(mode) = w.units.get(&key).map(|u| u.mode) else {
        return false;
    };
    let (ty, class, mode) = super::disguise::identity(w, inputs, key, mode);
    if ty != PLAYER || class != 6 || matches!(mode, 0 | 0x11) {
        return false;
    }
    let (Some(r), Some(l)) = (valid_at(w, key, 4), valid_at(w, key, 5)) else {
        return false;
    };
    let hand = |v: &super::items::ItemView| match (&w.item_tables.0, v.code) {
        (Some(t), Some(code)) => matches!(t.component(code), 5 | 6),
        _ => false,
    };
    (hand(&r) || hand(&l)) && of_type(w, &r, WEAP) && of_type(w, &l, WEAP)
}

/// The player request code 0x13 (the only code that keeps the
/// light and the two set-up calls).
pub const CODE_SEQUENCE: u8 = 0x13;

/// Fatal of an invalid player code.
pub const FATAL_PLAYER_CODE: u32 = 0x432;
/// Fatal of an object code other than 3 and 0x15.
pub const FATAL_OBJECT_CODE: u32 = 0x39C;

/// The mode request with (code, U, record, 1). A unit
/// not in S: nothing (the handlers run on a drained unit).
pub fn mode_request(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    code: u8,
    record: [i32; 7],
    out: &Outputs,
) -> Result<bool, HandlerError> {
    // A missile's request returns 1 at once (step 1): not stored.
    if key.unit_type == MISSILE {
        return Ok(true);
    }
    let late = w.late_input;
    let tick = w.server_ticks;
    let Some(u) = w.units.get_mut(&key) else {
        return Ok(true);
    };
    // Step 3.
    u.last_mode_request = Some(ModeRequest { code, record });
    u.request_late = late;
    u.mode_requests = u.mode_requests.wrapping_add(1);
    // The player and monster machines start a cast's client effects in
    // the request (codes 0x15 / 0x16): kept apart so a later request of
    // the same tick does not
    // hide it.
    if matches!(key.unit_type, PLAYER | MONSTER) && matches!(code, 0x15 | 0x16) {
        u.last_cast = Some(CastRequest {
            request: ModeRequest { code, record },
            count: u.mode_requests,
            late,
            tick,
        });
    }
    // Every player code but 0x13 resets the path and sets a mode;
    // a 0x13 leaves them to the request before.
    if key.unit_type == PLAYER && code != 0x13 {
        u.last_path_request = Some((ModeRequest { code, record }, u.mode_requests));
    }
    match key.unit_type {
        PLAYER => player(w, inputs, key, code, record),
        OBJECT => object(w, inputs, key, code, record, out).map(|()| true),
        ITEM => {
            item(w, key, code, record);
            Ok(true)
        }
        MONSTER => monster(w, inputs, key, code, record).map(|()| true),
        _ => Ok(true),
    }
}

/// The room-in-town test on U's room (the room list of
/// and the active room's level): a unit in no
/// room is not in town.
pub fn in_town(w: &ClientWorld, key: UnitKey) -> bool {
    let Some(room) = w.room_units.room_of(key) else {
        return false;
    };
    w.active_rooms
        .as_deref()
        .and_then(|rooms| rooms.iter().find(|r| r.room == room))
        .is_some_and(|r| d2_sim::drlg::is_town(u32::from(r.level)))
}

/// The level id of U's room: `None` in no
/// room.
pub fn level_of(w: &ClientWorld, key: UnitKey) -> Option<u16> {
    let room = w.room_units.room_of(key)?;
    w.active_rooms
        .as_deref()?
        .iter()
        .find(|r| r.room == room)
        .map(|r| r.level)
}

/// The neutral and walk modes of U: 5 / 6 in town, else 1 / 2.
pub fn neutral_walk(w: &ClientWorld, key: UnitKey) -> (u32, u32) {
    if in_town(w, key) {
        (player_mode::TOWN_NEUTRAL, player_mode::TOWN_WALK)
    } else {
        (player_mode::NEUTRAL, player_mode::WALK)
    }
}

/// The cast light remove with (U, 0): the unit's cast light (held in
/// its stat list) is detached and removed; a
/// unit without one: nothing. The unit's own light (`+0x64`, the player
/// light) stays.
pub(super) fn remove_unit_light(w: &mut ClientWorld, key: UnitKey) {
    if let Some(id) = w.cast_lights.remove(&key) {
        let _ = w.lights.remove(id);
    }
}

/// The skill cast light with (U, missile),
/// which the client skill start calls last when the
/// start passed and the skill's `cltmissile` (`+0xE8`) > 0: a missile
/// `Light` of 0 → nothing; else a kind-1 light of U (radius 1, `I` 255,
/// the missile's `Red`, `Green`, `Blue`, at U's sub-tile) with target
/// `Light`, attached to U; the cast light U held before
/// is removed. No light-quality test (unlike the
/// missile's own light).
pub(super) fn cast_light(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, missile: i32) {
    let Some(row) = usize::try_from(missile)
        .ok()
        .and_then(|m| inputs.tables.missiles.get(m))
    else {
        return;
    };
    let Some(req) =
        crate::rules::lighting::sources::skill_cast_light(i32::from(row.light), row.rgb)
    else {
        return;
    };
    let Some((x, y)) = w.units.get(&key).map(|u| u.cell()) else {
        return;
    };
    use crate::rules::lighting::records::{unit_light_pos, LightKind, Owner};
    let owner = Owner {
        unit_type: u32::from(key.unit_type),
        guid: key.guid,
        client_only: false,
    };
    let pos = (
        unit_light_pos(i32::from(x) << 16),
        unit_light_pos(i32::from(y) << 16),
    );
    let Some(id) = w.lights.create(
        Some(owner),
        pos,
        LightKind::Plain,
        req.radius,
        req.i,
        req.r,
        req.g,
        req.b,
    ) else {
        return;
    };
    if let Some(t) = req.target {
        w.lights.set_target(id, t);
    }
    if let Some(old) = w.cast_lights.insert(key, id) {
        let _ = w.lights.remove(old);
    }
}

/// Places U at (x, y): the room of the point (none is fatal 0x168),
/// the position, and the room list recache.
/// Without the client DRLG the point is
/// taken as in a room.
fn place(w: &mut ClientWorld, key: UnitKey, x: u16, y: u16) -> Result<(), HandlerError> {
    let room = match &w.active_rooms {
        Some(_) => Some(w.room_at(x, y).ok_or(HandlerError::Fatal(0x168))?),
        None => None,
    };
    if let Some(u) = w.units.get_mut(&key) {
        u.position = Some((x, y));
    }
    if w.active_rooms.is_some() {
        w.room_units.place(key, room.map(|r| r.room));
    }
    Ok(())
}

/// The player death that codes 0x08 and 0x09 end in:
/// of its steps the model holds stat 6 (life)
/// := 0; the dead flag 0x10000 is read from
/// the mode ([`super::world::ClientUnit::is_dead`]), and the path stop
/// and the flag 0x4 / 0x2 clears have no model field.
fn player_death(w: &mut ClientWorld, key: UnitKey) {
    w.units.get_mut(&key).expect("present").stats.insert(6, 0);
}

/// The player machine, with flag 1 (every
/// queued message passes 1).
fn player(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    code: u8,
    r: [i32; 7],
) -> Result<bool, HandlerError> {
    if matches!(code, 3..=5 | 0x0A..=0x11) || code > 0x19 {
        return Err(HandlerError::Fatal(FATAL_PLAYER_CODE));
    }
    remove_unit_light(w, key);
    // (unless code 0x13) are presentation
    // and used-skill resets of the client animation; no model field.
    let (neutral, walk) = neutral_walk(w, key);
    let u = w.units.get_mut(&key).expect("checked by the caller");
    // A unit in mode 0x13 with flag 1: its path is stopped;
    // then the path reset (no client path record).
    if u.mode == player_mode::SEQUENCE {
        u.path_stopped = true;
    }
    let (x, y) = (r[0] as u16, r[1] as u16);
    // A walk to a point puts the point on the path (target,
    // +0x10 / +0x12), the field a skill start writes.
    // `local_turn` (4 a client update from standing, 1 re-aiming, measured
    // on a long walk) is no longer read by the model: the turn
    // is the dirdiff step of the aim (one
    // per player update), through `Predict::new_direction`.
    if matches!(code, 0x00 | 0x01 | 0x17 | 0x18) && w.local_player == Some(key) {
        let walking = w
            .units
            .get(&key)
            .is_some_and(|u| matches!(u.mode, 2 | 3 | 6));
        // A walk to a unit turns 1 a step (recorded).
        w.local_turn = if walking || matches!(code, 0x00 | 0x18) {
            1
        } else {
            4
        };
        // A walk to a unit follows the unit: its point is the path's target.
        if matches!(code, 0x00 | 0x18) {
            let target = UnitKey::new(r[0] as u8, r[1] as u32);
            let at = w.units.get(&target).and_then(|t| t.position);
            if let Some(u) = w.units.get_mut(&key) {
                u.path_follow = Some(target);
                if at.is_some() {
                    u.path_target = at;
                }
            }
        }
    }
    if matches!(code, 0x01 | 0x17) {
        if let Some(u) = w.units.get_mut(&key) {
            u.path_target = Some((x, y));
            u.path_follow = None;
        }
    }
    // The mode set with the animation restart the client
    // player update ends the mode by ([`super::player_anim`]).
    let set = |w: &mut ClientWorld, m: u32| {
        super::player_anim::mode_set(w, inputs, key, m);
    };
    let hit = |w: &mut ClientWorld| {
        w.units.get_mut(&key).expect("present").hit_class = r[2] as u32;
    };
    match code {
        // (path to a unit / a point), then
        // mode := walk. PROVISIONAL (DIFFERS): the path helpers
        // succeed here (the client holds no path record, so the "helper
        // returned 0 → neutral" branch never runs). The bodies of
        // (read 2026-10-15) set the path target
        // (the unit, 0 when the target unit is not found;
        // the point) and return the client path compute;
        // 0 gives with (1) with no walk; fix:
        // q-fix-client-path-exact.
        0x00 | 0x01 => set(w, walk),
        // The interact sender (step 7) runs from the message handler
        // (`objects::interact::mode_request_code_2`): no mode change.
        0x02 => {}
        0x06 => {
            hit(w);
            set(w, player_mode::GET_HIT);
            check(w, inputs, key, x, y, 1, 0, 0)?;
        }
        0x07 => {
            // was-dead feed the re-init
            // with (U, was-dead).
            let u = w.units.get_mut(&key).expect("present");
            let was_dead = u.is_dead();
            u.flag_2 = Some(true);
            u.mode = neutral;
            // PROVISIONAL (D2RUST's own, unverified):
            // places a unit that was dead at its point, as 0x15's place
            // (without the free-point fallback).
            // The respawn's 0x0D code 7 (C→S 0x41) is the only message
            // that gives the client the town point: the 0x15 before it
            // finds the player dead and leaves it.
            if was_dead && (x, y) != (0, 0) {
                place(w, key, x, y)?;
            }
        }
        0x08 => {
            // The local player's UI resets (hover target)
            // belong to the UI layer.
            hit(w);
            check(w, inputs, key, x, y, 0, 0, 0)?;
            set(w, player_mode::DEATH);
            player_death(w, key);
        }
        0x09 => {
            set(w, player_mode::DEAD);
            player_death(w, key);
        }
        0x12 => {
            hit(w);
            // Mode 9 (BL) when the unit can block.
            if holds_shield(w, key) || holds_two_claws(w, inputs, key) {
                set(w, MODE_BLOCK_0X12);
            }
            check(w, inputs, key, x, y, 1, 0, 0)?;
        }
        0x13 => {
            hit(w);
            // The hit overlays with (U, 0x13); the sound
            // is the audio feed's.
            hit_overlays(w, inputs, key, 0x13);
            check(w, inputs, key, x, y, 1, 0, 0)?;
        }
        0x14 => {
            hit(w);
            // The knockback: path stop, type 0xB toward, then the
            // mode set 0x13 with its animation init (frame 0, KB's count),
            // which the player update holds and ends (mode 19; measured:
            // fr 0, fc 1280).
            let u = w.units.get_mut(&key).expect("present");
            u.path_stopped = true;
            u.path_live = None;
            set(w, player_mode::SEQUENCE);
        }
        // The client skill start after the
        // target fix-up: (cast light); the
        // skill's mode is the client animation's (open question 1).
        0x15 | 0x16 => {
            // The used skill of the start: the
            // entry of skill and owner, read by the
            // client do. The base game adds a
            // skill the list does not hold, then
            // uses it ([`start_entry`]); still none → the start returns 0
            // and the mode is left.
            // The dual-wield toggle before the start
            // (0x16 not for a monster).
            let mut r = r;
            if code == 0x15 || key.unit_type != MONSTER {
                r[0] = dual_wield_skill(w, key, r[0]);
            }
            if !start_entry(w, inputs, key, r[0] as u16, r[1] as u32, r[4]) {
                return Ok(false);
            }
            // The entry's mode: the Assassin's Left Hand Swing is S4
            // (the native add).
            let m = if key.unit_type == PLAYER
                && r[0] == 5
                && w.units.get(&key).is_some_and(|u| u.class == 6)
            {
                Some(MODE_S4)
            } else {
                skill_mode(inputs, r[0], PLAYER)
            };
            if let Some(m) = m {
                set(w, m);
            }
            // The start faces the cast point (0x15:) or the target
            // unit (0x16).
            let face = if code == 0x15 {
                u16::try_from(r[2]).ok().zip(u16::try_from(r[3]).ok())
            } else {
                w.units
                    .get(&UnitKey::new(r[2] as u8, r[3] as u32))
                    .and_then(|t| t.position)
            };
            if let Some((fx, fy)) = face {
                super::critter_path::face_point(w, key, fx, fy);
            }
            // The path keeps the cast target for the client do's
            // `cltmissile` create.
            let cast = if code == 0x15 {
                face.map(|(fx, fy)| CastTarget::Point(fx, fy))
            } else {
                Some(CastTarget::Unit(UnitKey::new(r[2] as u8, r[3] as u32)))
            };
            if let Some(u) = w.units.get_mut(&key) {
                u.cast_target = cast;
            }
            // A point start puts its point on the path as well, ending a
            // walk to a unit's follow (0x16 leaves the path target as is).
            if code == 0x15 {
                if let Some(f) = face {
                    if let Some(u) = w.units.get_mut(&key) {
                        u.path_target = Some(f);
                        u.path_follow = None;
                    }
                }
            }
            // Step 5: the `cltstfunc` returns 0 → current := none, mode
            // set 1, and the request returns 0. 0x15's start clears the
            // target; 0x16's is the unit of (type, GUID).
            let target = (code == 0x16).then(|| UnitKey::new(r[2] as u8, r[3] as u32));
            let skill = r[0] as u16;
            let passes = if inputs
                .tables
                .skills
                .get(usize::from(skill))
                .is_some_and(|row| row.cltstfunc == 19)
            {
                // Function 19 (Teeth) creates the
                // `cltmissilec` cast missile at the level with bonuses of
                // the used entry.
                let entry = w
                    .units
                    .get(&key)
                    .and_then(|u| u.skills.as_ref())
                    .and_then(|l| l.current.and_then(|i| l.entries.get(i)))
                    .copied();
                match entry {
                    Some(e) => {
                        let t = &inputs.tables;
                        let level = super::msg::skills::level_with_bonuses(
                            w,
                            key,
                            &t.skills,
                            &t.skilldesc,
                            &e,
                        );
                        super::client_do::bone_cast(w, inputs, key, skill, level)?
                    }
                    None => false,
                }
            } else {
                super::use_state::client_start_passes(w, inputs, key, skill, target)
            };
            if !passes {
                if let Some(l) = w.units.get_mut(&key).and_then(|u| u.skills.as_mut()) {
                    l.current = None;
                }
                set(w, player_mode::NEUTRAL);
                return Ok(false);
            }
            // The start's level L: the local
            // player's level with bonuses, else the request's.
            let level = if w.local_player == Some(key) {
                w.units
                    .get(&key)
                    .and_then(|u| u.skills.as_ref())
                    .and_then(|l| l.current.and_then(|i| l.entries.get(i)))
                    .copied()
                    .map_or(r[4], |e| {
                        let t = &inputs.tables;
                        super::msg::skills::level_with_bonuses(w, key, &t.skills, &t.skilldesc, &e)
                    })
            } else {
                r[4]
            };
            super::use_state::client_start_apply(w, inputs, key, r[0] as u16, level)?;
            // The start's last steps: the `castoverlay`
            // (in 1 … count − 1, none of the id
            // playing; only its light is modelled here), then the cast
            // light.
            if let Some(row) = inputs.tables.skills.get(r[0] as usize) {
                if let Ok(o) = u16::try_from(row.castoverlay) {
                    if o > 0 && !super::overlay_lights::has(w, key, o) {
                        super::overlay_lights::create(w, inputs, key, o, 2);
                    }
                }
                if row.cltmissile > 0 {
                    cast_light(w, inputs, key, i32::from(row.cltmissile));
                }
            }
        }
        0x17 | 0x18 => set(w, player_mode::RUN),
        0x19 => {
            set(w, player_mode::BLOCK);
            check(w, inputs, key, x, y, 0, 0, 0)?;
        }
        _ => unreachable!("fatal codes returned above"),
    }
    Ok(true)
}

/// The object machine: code 3 is the mode
/// change (is the mode), code 0x15 the
/// shrine use (run by the caller after this), anything else fatal.
fn object(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    code: u8,
    r: [i32; 7],
    out: &Outputs,
) -> Result<(), HandlerError> {
    match code {
        3 => {
            let mode = r[1] as u32;
            let u = w.units.get_mut(&key).expect("checked by the caller");
            u.mode = mode;
            // The mode change ends by writing flag
            // 0x20 from `BlocksLight[mode]`.
            if let KindData::Object(d) = &mut u.kind {
                d.shadow_mode = Some(mode);
            }
            let class = u.class;
            // The footprint follows the mode the server set
            // (`HasCollision[mode]`): a door opened by anyone other than
            // the local player is walkable to this client as well, and a
            // closed one blocks again.
            if let Some(row) = inputs.objclient.rows.get(class as usize) {
                if let KindData::Object(d) = &mut u.kind {
                    d.footprint = row.shape.collides_in(mode);
                }
            }
            // The graphics refresh (after `set_mode`, not for
            // a door): the new mode needs its `Mode<m>` flag, else fatal
            // 0x4DC.
            if let Some(row) = inputs.objclient.rows.get(class as usize) {
                if row.is_door == 0 && row.mode_ok.get(mode as usize) != Some(&1) {
                    return Err(HandlerError::Fatal(super::objects::FATAL_GFX_MODE));
                }
            }
            // The animation set-up of the new mode (measured: the base
            // game's runs
            // itself after its `set_mode`, drawing again even
            // in the same mode, measured on the town objects). Nothing without rows.
            if let Some(row) = inputs.objclient.rows.get(class as usize) {
                super::objects::anim_setup(u, row, mode)?;
            }
            // The object light of the new mode (`Lit<mode>` / 2). Without
            // the class's row the
            // light cannot be read: nothing.
            if let Some(row) = inputs.tables.objects.get(class as usize).copied() {
                let lit = *row.lit.get(mode as usize).ok_or(HandlerError::Invalid(
                    "object mode past the eight objects.txt modes",
                ))?;
                object_light(w, key, lit, row.rgb);
            }
            // The mode sound call inside the change.
            let unit = ObjUnit {
                key,
                client_only: false,
            };
            out.push(Output::ObjectSound(ObjSound::Mode {
                unit,
                class,
                mode,
                local_dist: super::objects::local_distance(w, inputs, unit),
            }));
            Ok(())
        }
        0x15 => Ok(()),
        _ => Err(HandlerError::Fatal(FATAL_OBJECT_CODE)),
    }
}

/// The item machine.
fn item(w: &mut ClientWorld, key: UnitKey, code: u8, r: [i32; 7]) {
    if code != 2 {
        return;
    }
    let u = w.units.get_mut(&key).expect("checked by the caller");
    u.mode = r[1] as u32;
    u.flag_2 = Some(r[0] != 0);
}

/// The mode of the client skill start of codes 0x15 / 0x16 (S→C 0x4D /
/// 0x4C: record[0] = the skill id): the
/// skill's `skills.txt` `anim` (player modes) or `monanim` (monster
/// modes); a skill without a row or a mode past the type's table:
/// `None` (mode unchanged).
///
/// The face that follows for a monster (0x15 the point, 0x16 the unit)
/// is measured.
/// The client skill start is specified
/// (measured on a fire tower and a Teeth cast); this is its step 3 for a
/// skill other than 0, the entry's
/// mode. PROVISIONAL: not run here are the player's attack pick
/// for skill 0 (one draw of the client seed) and the m = 0
/// neutral set (step 4); settled by a capture of the client state over
/// four Attack starts.
pub fn skill_mode(inputs: &ModelInputs, skill: i32, unit_type: u8) -> Option<u32> {
    let row = inputs.tables.skills.get(usize::try_from(skill).ok()?)?;
    let (m, count) = if unit_type == PLAYER {
        (row.anim, 20)
    } else {
        (row.monanim, MODE_ROWS.len() as u8)
    };
    (m < count).then_some(u32::from(m))
}

/// Monster modes of the machine.
pub mod monster_mode {
    pub const DEATH: u32 = 0;
    pub const NEUTRAL: u32 = 1;
    pub const WALK: u32 = 2;
    pub const GET_HIT: u32 = 3;
    pub const ATTACK1: u32 = 4;
    pub const ATTACK2: u32 = 5;
    pub const BLOCK: u32 = 6;
    pub const CAST: u32 = 7;
    pub const SKILL1: u32 = 8;
    pub const SKILL2: u32 = 9;
    pub const SKILL3: u32 = 10;
    pub const SKILL4: u32 = 11;
    pub const DEAD: u32 = 12;
    pub const KNOCKBACK: u32 = 13;
    pub const SEQUENCE: u32 = 14;
    pub const RUN: u32 = 15;
}

/// The mode table of the
/// point and unit groups: the mode their code sets.
pub fn monster_table_mode(code: u8) -> Option<u32> {
    use monster_mode as m;
    Some(match code {
        0x04 | 0x05 => m::CAST,
        0x0A | 0x0B => m::ATTACK1,
        0x0C | 0x0D => m::SKILL1,
        0x0E | 0x0F => m::SKILL2,
        0x10 | 0x11 => m::ATTACK2,
        0x1A | 0x1B => m::SKILL3,
        0x1C | 0x1D => m::SKILL4,
        _ => return None,
    })
}

/// With (U, 0): U faces its target unit, the centre of the
/// target's cell (the turn step); a target
/// that is U itself is not faced.
fn face_unit(w: &mut ClientWorld, key: UnitKey, target: UnitKey) {
    if target == key {
        return;
    }
    let Some(t) = w.units.get(&target) else {
        return;
    };
    let (tx, ty) = match w.local_predicted.filter(|_| w.local_player == Some(target)) {
        // The local player stands where it walked to, not at the last
        // point the server gave it (B025).
        Some((px, py)) => ((px >> 16) as u16, (py >> 16) as u16),
        None => t.cell(),
    };
    super::critter_path::face_point(w, key, tx, ty);
}

/// The neutral fallback "F": a monster in mode
/// 1…15 other than 12 gets the path stop (precise position to its cell's
/// centre) and is set to mode 1; any other mode: nothing.
fn neutral_fallback(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) {
    let mode = w.units.get(&key).expect("checked by the caller").mode;
    if (1..=15).contains(&mode) && mode != monster_mode::DEAD {
        // first faces the path's target unit
        // from the precise point before the stop centres it (measured: a
        // Fallen whose walk to the player ends in
        // code 7 turns 58 → 56 in NU).
        super::critter_path::face_target(w, key);
        // The path stop ->: the path's precise
        // position becomes the centre of the cell it is in (`(p &
        // 0xFFFF0000) + 0x8000` per axis), so a walker that ended short
        // of its goal's centre is drawn at the centre from here on.
        if let Some(u) = w.units.get_mut(&key) {
            if let Some((px, py)) = u.precise {
                u.precise = Some(((px & 0xFFFF_0000) | 0x8000, (py & 0xFFFF_0000) | 0x8000));
            }
        }
        super::monster_anim::mode_set(w, inputs, key, monster_mode::NEUTRAL);
    }
}

/// A monster's stop walk reached the stop point: the path stop, then
/// neutral (the model's walk has no end of its own).
pub(super) fn end_walk(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) {
    if w.units.get(&key).is_some_and(|u| {
        u.key.unit_type == MONSTER && matches!(u.mode, monster_mode::WALK | monster_mode::RUN)
    }) {
        neutral_fallback(w, inputs, key);
    }
}

/// The life byte of codes 6 and 0x13: the low 7 bits, one
/// more when above 1.
/// With (U): the hit flash counter U +0x58 := 3, after the blood
/// spray of the monster codes 0x06 (with or without the spray) and 0x13.
fn arm_hit_flash(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey) {
    if let Some(u) = w.units.get_mut(&key) {
        u.hit_flash = 3;
        u.hit_flash_interact = inputs
            .tables
            .monsters
            .get(u.class as usize)
            .and_then(|c| c.as_ref())
            .is_some_and(|c| c.interact);
    }
}

fn life_byte(v: i32) -> i32 {
    let l = v & 0x7F;
    if l > 1 {
        l + 1
    } else {
        l
    }
}

/// The monster machine,
/// mode column: the class gate of the head (a class outside `monstats`
/// or without a `monstats2` row: no switch), then the dispatch by code.
/// The path, light, overlay and stat effects of the rows belong to their
/// own seams; NPC busy (monster data +0x28 bit 0) and the BaseId cases
/// `iceglobe` and `vulture1` read fields the model does not hold and are
/// not taken here.
///
/// The client path helpers (to a unit, to a
/// point) compute on the monster's client path (`critter_path`), which
/// a monster without one (no client DRLG) takes as found.
/// Code 7's "class has mode 2" test with (class, 2) reads the
/// class's `monstats2` row (`monstats` +0x18) mode bits at +0xF0, bit 2
/// (`mWL`; [`super::world::MonsterClass::modes`]): a class without a
/// walk mode takes F.
fn monster(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    code: u8,
    r: [i32; 7],
) -> Result<(), HandlerError> {
    use monster_mode as m;
    // head: a dead U and code ∉ {8, 9} → the
    // monster light re-created.
    if !matches!(code, 0x08 | 0x09) && w.units[&key].is_dead() {
        let class = w.units[&key].class as usize;
        if let Some(Some(row)) = inputs.tables.monsters.get(class) {
            let row = *row;
            super::msg::lighting::monster_light(w, key, &row);
        }
    }
    // head, pathed request: > 0 → path type :=.
    // A 0x67 walk carries its path type here (Warriv's
    // type 7 makes a blocked goal no path: F, not a walk).
    // Then (also for r3 <= 0) the used skill := none:
    // a later attack's event byte runs no skill do (measured: the A1
    // after the ArcaneTower
    // cast makes no second ring).
    if !matches!(code, 0x13 | 0x15 | 0x16) {
        if r[3] > 0 {
            super::critter_path::server_path_type(w, key, r[3] as u32);
        }
        w.units
            .get_mut(&key)
            .expect("checked by the caller")
            .used_skill = None;
    }
    // Then (also for r3 ≤ 0) current skill := none:
    // the do of the event-byte step (`monster_anim::skill_step`) reads it.
    if !matches!(code, 0x13 | 0x15 | 0x16) {
        if let Some(u) = w.units.get_mut(&key) {
            u.used_skill = None;
        }
    }
    // class gate.
    let class = w.units[&key].class;
    if !inputs
        .tables
        .monsters
        .get(class as usize)
        .is_some_and(|c| c.is_some())
    {
        return Ok(());
    }
    // The mode set restarts the unit's animation (`monster_anim`).
    let set = |w: &mut ClientWorld, mode: u32| {
        super::monster_anim::mode_set(w, inputs, key, mode);
        super::critter_path::server_velocity(w, inputs, key);
    };
    // The monster's client path (target :=...).
    let server = super::critter_path::Who::server(key);
    // +0xB0 (for 0x06 / 0x14, for 0x13).
    let hit = |w: &mut ClientWorld, h: i32| {
        w.units.get_mut(&key).expect("present").hit_class = h as u32;
    };
    match code {
        // Path to the unit (type, GUID): an absent unit → F.
        0x00 | 0x18 => {
            let target = UnitKey::new(r[0] as u8, r[1] as u32);
            if w.units.contains_key(&target)
                && super::critter_path::server_walk_unit(w, inputs, key, target)
            {
                set(w, if code == 0x00 { m::WALK } else { m::RUN });
            } else {
                neutral_fallback(w, inputs, key);
            }
        }
        // Path to the point: no path → F.
        0x01 | 0x17 => {
            if super::critter_path::server_walk(w, inputs, key, r[0], r[1]) {
                set(w, if code == 0x01 { m::WALK } else { m::RUN });
            } else {
                neutral_fallback(w, inputs, key);
            }
        }
        // Point group: face when it is a point other than the
        // unit's own, then the mode.
        0x04 | 0x0B | 0x0C | 0x0E | 0x11 | 0x1A | 0x1C => {
            super::critter_path::set_target_unit(w, server, None);
            let (x, y) = ((r[0] & 0xFFFF) as u16, (r[1] & 0xFFFF) as u16);
            if (r[0] != 0 || r[1] != 0) && w.units[&key].cell() != (x, y) {
                super::critter_path::face_point(w, key, x, y);
            }
            set(w, monster_table_mode(code).expect("table code"));
        }
        // Unit group: the target face with (U, 0) faces the target unit (the path
        // target's cell centre, the turn step), then the mode.
        0x05 | 0x0A | 0x0D | 0x0F | 0x10 | 0x1B | 0x1D => {
            let target = UnitKey::new(r[0] as u8, r[1] as u32);
            super::critter_path::set_target_unit(w, server, Some(target));
            if super::critter_path::target_unit(w, server).is_some() {
                super::critter_path::face_target(w, key);
            } else {
                face_unit(w, key, target);
            }
            set(w, monster_table_mode(code).expect("table code"));
        }
        // code 6: life byte L := & 0x7F (L > 1 → L + 1);
        // L ≠ 0 equal to stat 6 >> 8 → no spray, else stat 6 := L << 8.
        // After the get-hit mode set, the blood spray with (U, 1,
        // 0).
        0x06 => {
            let life = life_byte(r[2]);
            let mut bleed = true;
            if life != 0 {
                let u = w.units.get_mut(&key).expect("present");
                if u.stat(6) >> 8 == life {
                    bleed = false;
                } else {
                    u.stats.insert(6, life << 8);
                }
            }
            hit(w, r[6]);
            set(w, m::GET_HIT);
            if bleed {
                super::blood_spray::spray(w, inputs, key, 1, false)?;
            }
            arm_hit_flash(w, inputs, key);
        }
        0x07 => {
            // code 7: flag 0x20 := not `monstats2` `shadow`
            // (the only request that clears a held death's no-shadow
            // flag).
            if let Some(s) = inputs
                .tables
                .monsters
                .get(class as usize)
                .and_then(|c| c.as_ref())
                .and_then(|c| c.setup.as_ref())
            {
                w.units.get_mut(&key).expect("present").flag_20 = !s.shadow;
            }
            // A record with life byte ≠ 0 sets
            // stat 6 := << 8 (the 0x6D hireling update of a hireling
            // at 0x80 shows half of the 0xFC00 the stat messages gave).
            if r[2] != 0 {
                w.units
                    .get_mut(&key)
                    .expect("present")
                    .stats
                    .insert(6, r[2] << 8);
            }
            // Position check (kind 0), then within 1 sub-tile of
            // → F; else walk (state 143 `attached` → F).
            let (x, y) = ((r[0] & 0xFFFF) as u16, (r[1] & 0xFFFF) as u16);
            check(w, inputs, key, x, y, 0, 0, 0)?;
            let u = &w.units[&key];
            let (cx, cy) = u.cell();
            let near = (i32::from(cx) - i32::from(x)).abs() <= 1
                && (i32::from(cy) - i32::from(y)).abs() <= 1;
            // The class mode test with 2: the class's `mWL` bit.
            let walks = inputs
                .tables
                .monsters
                .get(class as usize)
                .and_then(|c| c.as_ref())
                .is_some_and(|c| c.modes & (1 << m::WALK) != 0);
            if near || u.states.contains(&143) || !walks {
                neutral_fallback(w, inputs, key);
            } else {
                set(w, m::WALK);
            }
        }
        // Step 5: +0xB0 := (the hit class the death
        // sound's impact pair reads), stat 6
        // := 0, then the switch by old BaseId ([`death_switch`]).
        0x08 => {
            hit(w, r[6]);
            w.units.get_mut(&key).expect("present").stats.insert(6, 0);
            death_groups(w, inputs, key, class)?;
            let base_id = inputs.tables.monsters[class as usize]
                .as_ref()
                .map_or(0, |c| c.base_id);
            death_switch(w, inputs, key, base_id)?;
        }
        // Mode := 0xC (the Maggot Queen in her
        // death sequence excepted); then step 5.3.
        0x09 => {
            let base_id = inputs.tables.monsters[class as usize]
                .as_ref()
                .map_or(0, |c| c.base_id);
            // Base id 284 `maggotqueen1` in mode 0xE keeps
            // its death sequence: the used skill := its QueenDeath (216)
            // entry, no mode set, no dead
            // flag; the death extras still run.
            if base_id == 284 && w.units[&key].mode == m::SEQUENCE {
                let u = w.units.get_mut(&key).expect("present");
                u.used_skill = Some(216);
                if let Some(l) = u.skills.as_mut() {
                    l.current = l.native(216);
                }
            } else {
                set(w, m::DEAD);
            }
            dead_extras(w, inputs, key, base_id)?;
        }
        0x12 => {
            let target = UnitKey::new(r[0] as u8, r[1] as u32);
            super::critter_path::set_target_unit(w, server, Some(target));
            set(w, m::BLOCK);
        }
        // No mode change (the KB mode comes with 0x14); the mode sound
        // with (U, 0xD, 1) is the audio feed's. Life byte L :=
        // as code 6, L ≠ 0 → stat 6 := L << 8; then the rolled blood
        // spray with (U, 1, 1) and the hit overlays
        // with (U, 0xD).
        0x13 => {
            let life = life_byte(r[1]);
            if life != 0 {
                w.units
                    .get_mut(&key)
                    .expect("present")
                    .stats
                    .insert(6, life << 8);
            }
            hit(w, r[0]);
            super::blood_spray::spray(w, inputs, key, 1, true)?;
            arm_hit_flash(w, inputs, key);
            hit_overlays(w, inputs, key, m::KNOCKBACK);
        }
        // row 0x14: ≠ 0 → stat 6 := << 8 (no life-byte
        // transform), so the code 6 that ends the knockback compares its
        // life byte against it and skips the spray when equal (measured:
        // no blood).
        0x14 => {
            if r[5] != 0 {
                w.units
                    .get_mut(&key)
                    .expect("present")
                    .stats
                    .insert(6, r[5] << 8);
            }
            hit(w, r[6]);
            // The KB path set-up's compute faces away from the KB point
            // (`critter_path::face_knockback`).
            super::critter_path::face_knockback(
                w,
                key,
                (r[0] & 0xFFFF) as u16,
                (r[1] & 0xFFFF) as u16,
            );
            set(w, m::KNOCKBACK);
        }
        // 0x15 / 0x16: the skill is added at level
        // and becomes the used skill; then
        // the mode set of the entry's mode, 0xE (sequence)
        // included: its sequence load reads the used skill
        // (`monster_anim::mode_init`).
        //
        // Then the client skill start wrappers: 0x16's
        // looks up the target unit (type, GUID):
        // found → target := it and the target face with (U, 0) faces it; absent →
        // code 7 with a null record, the neutral fallback F. 0x15's
        // sets the path target point, faces it and
        // clears the target. The client do's `cltmissile` aims at that
        // target (`client_do`).
        0x15 | 0x16 => {
            if code == 0x15 {
                super::critter_path::set_target_unit(w, server, None);
            }
            let u = w.units.get_mut(&key).expect("present");
            u.used_skill = Some(r[0]);
            u.used_skill_level = r[4];
            // The add at level and the current entry:
            // a monster's do functions
            // read that entry's flags and params. Without a list: nothing.
            if let (Ok(skill), Some(list)) = (u16::try_from(r[0]), u.skills.as_mut()) {
                let owner = super::skills::Owner {
                    unit_type: MONSTER,
                    class: u.class,
                };
                let rows = &inputs.tables.skills;
                let done = super::skills::assign(list, rows, owner, skill, r[4], false);
                list.current = list.native(skill);
                let fx = std::mem::take(&mut list.fx);
                done.map_err(HandlerError::from)?;
                super::passive::apply(w, inputs, key, fx)?;
            }
            // The mode is the skill entry's +0x08: the
            // monster's own entry keeps the `SkNmode` its set-up gave it
            // an entry the add creates takes `monanim`.
            let own = u16::try_from(r[0]).ok().and_then(|s| {
                let l = w.units[&key].skills.as_ref()?;
                l.native(s).map(|i| l.entries[i].mode)
            });
            let mode = match own {
                Some(m) => (m < MODE_ROWS.len() as u32).then_some(m),
                None => skill_mode(inputs, r[0], MONSTER),
            };
            // A sequence request always reloads the sequence, the same mode
            // too (the preload, `monster_anim::mode_set_sequence`).
            match mode {
                Some(m::SEQUENCE) => {
                    super::monster_anim::mode_set_sequence(w, inputs, key, m::SEQUENCE);
                    super::critter_path::server_velocity(w, inputs, key);
                }
                Some(mode) => set(w, mode),
                None => {}
            }
            // After the mode set, the client skill start faces the cast:
            // 0x16 the unit (type, GUID), made the path
            // target, or the neutral fallback when the
            // unit is absent; 0x15 the point.
            // The update turns the monster by the step
            // (measured on a Fallen shaman: 59 → 60 → 61 over
            // its first two A2 frames).
            if code == 0x16 {
                let target = UnitKey::new(r[2] as u8, r[3] as u32);
                if w.units.contains_key(&target) {
                    super::critter_path::set_target_unit(w, server, Some(target));
                    if super::critter_path::target_unit(w, server).is_some() {
                        super::critter_path::face_target(w, key);
                    } else {
                        // No client path context to hold the target:
                        // face its cell directly.
                        face_unit(w, key, target);
                    }
                } else {
                    neutral_fallback(w, inputs, key);
                    return Ok(());
                }
            } else {
                let (x, y) = ((r[2] & 0xFFFF) as u16, (r[3] & 0xFFFF) as u16);
                super::critter_path::set_target(w, server, x, y);
                super::critter_path::face_point(w, key, x, y);
                // A monster of `Code` `PB  ` (the turrets, `firetower`)
                // then turns its new direction by +0x38.
                let pb = inputs
                    .tables
                    .monsters
                    .get(w.units[&key].class as usize)
                    .copied()
                    .flatten()
                    .is_some_and(|r| r.code == *b"PB  ");
                if key.unit_type == MONSTER && pb {
                    super::critter_path::turret_turn(w, key);
                }
                // Then the path's target unit (+0x58) is cleared
                // a point cast has none (measured: the ArcaneTower do
                // finds no target unit and makes no ring).
                super::critter_path::set_target_unit(w, server, None);
            }
            // The start's `cltstfunc` step
            // (`client_do::monster_start`); 0x16 without its target unit
            // returned above (no start).
            let ran = super::client_do::monster_start(w, inputs, key, r[0] as u16, r[4])?;
            // Then the start's last step, as for a player: a start that
            // ran with `cltmissile` > 0 makes the cast light
            // (measured: the Fallen shaman's kind-1 light at the list
            // head).
            let missile = inputs
                .tables
                .skills
                .get(r[0] as usize)
                .map_or(0, |row| row.cltmissile);
            if ran != 0 && missile > 0 {
                cast_light(w, inputs, key, i32::from(missile));
            }
        }
        // 0x02, 0x03, 0x19 and unknown codes.
        _ => neutral_fallback(w, inputs, key),
    }
    // The tail of a pathed request (a record and a
    // code other than 0x13, 0x15, 0x16): stat 67 from r4.
    if !matches!(code, 0x13 | 0x15 | 0x16) {
        super::monster_anim::velocity_tail(w, inputs, key, r[4]);
        super::critter_path::server_velocity(w, inputs, key);
    }
    // Then the tail's umod phase 1,
    // on every branch that reaches it: a 284 death runs it
    // twice, once in its nested 0x15 and once here.
    umod_phase1(w, inputs, key)
}

/// The client umod hooks of phase 1 with (U, 0, 1):
/// only umod 22 `questcomplete` has one,
/// by U's class and mode after the case. Class 284
/// `maggotqueen1` in mode 14: one client missile 353
/// `queendeathcenter` at U's cell, owner the local player
/// (none → nothing), flags 1, skill 0, level 1.
/// PROVISIONAL (umod 22, mode 0): the mode-0
/// bosses' quest effects (Andariel 156 … Baal crab 544) are not modelled;
/// settled by a check that kills one of them.
fn umod_phase1(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Result<(), HandlerError> {
    const QUEST_COMPLETE: u32 = 0x004A_D2F0;
    const MAGGOT_QUEEN: u32 = 284;
    const QUEEN_DEATH_CENTRE: u32 = 353;
    let Some(u) = w.units.get(&key) else {
        return Ok(());
    };
    let KindData::Monster(d) = &u.kind else {
        return Ok(());
    };
    let Some(hooks) = crate::rules::umod_hooks::hooks_for(&d.umods, 1) else {
        return Ok(());
    };
    let (class, mode, (x, y)) = (u.class, u.mode, u.cell());
    for (_, hook) in hooks {
        if hook != QUEST_COMPLETE || class != MAGGOT_QUEEN || mode != monster_mode::SEQUENCE {
            continue;
        }
        let Some(player) = w.local_player else {
            continue;
        };
        let rec = super::client_missiles::CreateRecord {
            flags: super::client_missiles::flag::POSITION,
            owner: Some(player),
            class: QUEEN_DEATH_CENTRE,
            x: i32::from(x),
            y: i32::from(y),
            level: 1,
            ..Default::default()
        };
        super::client_missiles::create(
            w,
            &inputs.tables.missiles,
            &rec,
            inputs.high_light_quality,
        )?;
    }
    Ok(())
}

/// The overlay create with (U, id, 0) of the death switch: kind 2,
/// no arguments (with (U, id, 2, 0, 0, 0, 0, 0)), recorded for the
/// effect layer, its light made here. Its sounds (ids 0x97 / 0x98) are
/// the audio layer's.
fn death_overlay(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, id: u16) {
    w.hit_overlays.push(super::hit_overlays::HitOverlay {
        tick: w.server_ticks,
        unit: key,
        id,
        kind: 2,
        args: [0; 3],
        remove: false,
    });
    super::overlay_lights::create(w, inputs, key, id, 2);
}

/// The client missile create at U with (U, class, x, y, 0, 1):
/// record flags 1, start U's sub-tile, skill
/// 0, level 1, no target.
fn death_missile(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    class: u32,
) -> Result<(), HandlerError> {
    use super::client_missiles::{create, flag, CreateRecord};
    let (x, y) = w.units[&key].cell();
    let rec = CreateRecord {
        flags: flag::POSITION,
        owner: Some(key),
        class,
        x: i32::from(x),
        y: i32::from(y),
        level: 1,
        owner_dir64: super::client_do::owner_dir64(w, key),
        ..CreateRecord::default()
    };
    create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    Ok(())
}

/// Step 5's state groups before the switch:
/// U in a state of the `hide` group →
/// flag-ex |= 0x40000 (the unit draw skips it),
/// flags := (flags & ~2) | 0x20; U in a state of the `shatter`
/// group → the shatter missiles (state 184
/// `uberminion` without state 1 `freeze`: client missiles 427, 426; else
/// 271 + `monstats2` `Height`, then 275 `icebreaksmoke`) and +0xB0 :=
/// (+0xB0 & 0xF) | 0xA0. A Fallen killed by Frost Nova with state 107
/// `shatter` on leaves no corpse (a recorded Frost Nova hit, frame 276:
/// 1:33 gone, 3:320 `icebreaksmall`, 3:321 `icebreaksmoke`).
fn death_groups(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    class: u32,
) -> Result<(), HandlerError> {
    let in_group = |w: &ClientWorld, f: fn(&super::world::StateRow) -> bool| {
        w.units[&key]
            .states
            .iter()
            .any(|&s| inputs.tables.states.get(usize::from(s)).is_some_and(f))
    };
    if in_group(w, |r| r.hide) {
        let u = w.units.get_mut(&key).expect("present");
        u.flag_ex |= 0x40000;
        u.flag_2 = Some(false);
        u.flag_20 = true;
    }
    if in_group(w, |r| r.shatter) {
        let states = &w.units[&key].states;
        let classes: [u32; 2] = if states.contains(&184) && !states.contains(&1) {
            [427, 426]
        } else {
            let height = inputs
                .tables
                .monsters
                .get(class as usize)
                .and_then(|c| c.as_ref())
                .map_or(0, |c| u32::from(c.height));
            [271 + height, 275]
        };
        // One record {flags 0x2020 (random direction,
        // absolute target 0, 0), owner and origin U, level 1} through the
        // client missile create, class by class.
        use super::client_missiles::{create, flag, CreateRecord};
        for class in classes {
            let rec = CreateRecord {
                flags: flag::RANDOM_DIRECTION | flag::TARGET_ABSOLUTE,
                owner: Some(key),
                origin: Some(key),
                class,
                level: 1,
                owner_dir64: super::client_do::owner_dir64(w, key),
                ..CreateRecord::default()
            };
            create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
        }
        let u = w.units.get_mut(&key).expect("present");
        u.hit_class = (u.hit_class & 0xF) | 0xA0;
    }
    Ok(())
}

/// The death switch by old BaseId. Default D0: mode set 0,
/// flags &= ~2 (the selectable bit), the unit's
/// light removed; every
/// branch ends with the flag and the light. The kind-2 overlays
/// come before D0's mode set. The sounds are the audio
/// layer's.
fn death_switch(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    base_id: u16,
) -> Result<(), HandlerError> {
    use monster_mode as m;
    let set = |w: &mut ClientWorld, mode: u32| {
        super::monster_anim::mode_set(w, inputs, key, mode);
        super::critter_path::server_velocity(w, inputs, key);
    };
    let class = w.units[&key].class;
    let overlays: &[u16] = match base_id {
        127 => &[79],
        238 => &[78],
        247 => &[82],
        258 | 261 => &[157],
        118 => &[184],
        310..=312 => &[187],
        212 => &[188, 189],
        360 | 558 => &[185],
        417 | 418 => &[271],
        469 | 474 => &[229],
        497 => match class {
            497 | 499 => &[225],
            498 => &[226],
            _ => &[],
        },
        461 => &[204],
        _ => &[],
    };
    for &id in overlays {
        death_overlay(w, inputs, key, id);
    }
    // The mode D0 sets, or none when the branch set its own.
    let mode = match base_id {
        // Overlay 146, mode set 0; with a motion record
        // the fall motion follows, which the model does not run.
        110 => {
            death_overlay(w, inputs, key, 146);
            set(w, m::DEATH);
            None
        }
        // 190 `maggotegg1` in mode 0xE: the quiet mode set
        // (no hit overlays, no sound), death from frame 14 (+0x44 :=
        // 0xE00; measured on a sand maggot).
        190 if w.units[&key].mode == m::SEQUENCE => {
            w.units.get_mut(&key).expect("present").flag_40 = false;
            super::monster_anim::mode_init(w, inputs, key, m::DEATH);
            super::critter_path::server_velocity(w, inputs, key);
            w.units.get_mut(&key).expect("present").frame = 0xE00;
            None
        }
        // 284 `maggotqueen1`: this machine with code 0x15 and the record
        // {216 `QueenDeath`, 0, 0, 0, 1, 0, 0}, so its mode is the skill's.
        284 => {
            monster(w, inputs, key, 0x15, [216, 0, 0, 0, 1, 0, 0])?;
            None
        }
        // Client missile 470 at U, then mode set 0 (and sound 790).
        425..=427 => {
            death_missile(w, inputs, key, 470)?;
            Some(m::DEATH)
        }
        // 570 `baalclone`: client missile 614 at U, then as 559
        // `baalcrabstairs`: flags-ex |= 0x40000 (no overlay creates from
        // here on), mode set 12 (DD).
        559 | 570 => {
            if base_id == 570 {
                death_missile(w, inputs, key, 614)?;
            }
            w.units.get_mut(&key).expect("present").flag_ex |= 0x40000;
            Some(m::DEAD)
        }
        // Client missile 328 `dopplezonexplode` toward the cast target
        // (the client missile create with (U, 328, 0, 1)).
        356 => {
            super::client_do::cast_toward(w, inputs, key, 328, 0, 1)?;
            Some(m::DEATH)
        }
        // Every overlay removed, then overlay class − 173
        // (178–180 `hydra_end1`–`3`).
        351..=353 => {
            super::overlay_lights::remove_all(w, key);
            if let Some(id) = class.checked_sub(173).and_then(|c| u16::try_from(c).ok()) {
                death_overlay(w, inputs, key, id);
            }
            Some(m::DEATH)
        }
        // Direction := (direction + 0x20) & 0x3F.
        441 => {
            if let Some(d) = super::client_do::owner_dir64(w, key) {
                super::critter_path::face_direction(w, key, d.wrapping_add(0x20));
            }
            Some(m::DEATH)
        }
        _ => Some(m::DEATH),
    };
    if let Some(mode) = mode {
        set(w, mode);
    }
    w.units.get_mut(&key).expect("present").flag_2 = Some(false);
    super::msg::lighting::remove_own_light(w, key);
    Ok(())
}

/// Step 5.3, code 9 after the mode set: unless class 211
/// `duriel` or BaseId 497, 417, 418, every overlay removed;
/// BaseId 435 `barricadetower` → client missile 424 `tower death` at U;
/// BaseId 344, 559, 570 → flags-ex |= 0x40000; flags &= ~2; the unit's
/// light removed.
fn dead_extras(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    base_id: u16,
) -> Result<(), HandlerError> {
    if w.units[&key].class != 211 && !matches!(base_id, 497 | 417 | 418) {
        super::overlay_lights::remove_all(w, key);
    }
    if base_id == 435 {
        death_missile(w, inputs, key, 424)?;
    }
    let u = w.units.get_mut(&key).expect("present");
    if matches!(base_id, 344 | 559 | 570) {
        u.flag_ex |= 0x40000;
    }
    u.flag_2 = Some(false);
    super::msg::lighting::remove_own_light(w, key);
    Ok(())
}

/// The hit overlays with (U, m): a player in mode 0, 4 or
/// 0x13 or a monster in mode 0, 3 or 0xD whose +0xB0 (last hit class) has
/// a non-zero low nibble gets the overlay its high nibble names, a kind-2
/// create with (U, id, 2, 0, 0, 0, 0, 0):
/// its light here, the record for the effect layer as `last_hit_overlay`.
pub fn hit_overlays(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, m: u32) {
    let tick = w.server_ticks;
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    let modes: &[u32] = match key.unit_type {
        PLAYER => &[0, 4, 0x13],
        MONSTER => &[0, 3, 0xD],
        _ => return,
    };
    if !modes.contains(&m) || u.hit_class & 0xF == 0 {
        return;
    }
    let id = match u.hit_class & 0xF0 {
        0x10 => 54,
        0x20 => 81,
        0x30 => 36,
        0x40 => 1,
        0x70 => 147,
        0x80 => 21,
        0x90 => 113,
        _ => return,
    };
    // The create's own tests run on both halves:
    // the light's in `overlay_lights::create`, the record's in the layer.
    u.hit_overlays = u.hit_overlays.wrapping_add(1);
    u.last_hit_overlay = Some(super::world::HitOverlay {
        id,
        count: u.hit_overlays,
        tick,
    });
    super::overlay_lights::create(w, inputs, key, id, 2);
}

/// The client's dual-wield toggle (as on the server): an Attack (skill 0)
/// start of a dual-capable unit whose body locations 4
/// and 5 both hold a usable item of type 45 `weap` whose
/// own type is not 38: the Attack entry's (owner −1) +0x24 flips; from
/// 0 it becomes 5 and the start uses skill 5 (Left Hand Swing), else 0
/// and Attack. Measured: w2-14-bar-s3-spam, w2-14-ass-s4-spam (the left
/// hand swings first). Any other start keeps its skill.
fn dual_wield_skill(w: &mut ClientWorld, key: UnitKey, skill: i32) -> i32 {
    if skill != 0 || !super::attack_weapon::dual_capable(w, key) {
        return skill;
    }
    let weapon = |l: u8| {
        super::attack_weapon::worn_at(w, key, l).is_some_and(|v| {
            let (weap, own) = match (&w.item_tables.0, v.code) {
                (Some(t), Some(code)) => (
                    t.item_is_type(code, WEAP),
                    t.missile_and_type(code).map(|(_, t)| t),
                ),
                _ => (false, None),
            };
            valid_item(w, &v) && weap && own != Some(38)
        })
    };
    if !(weapon(4) && weapon(5)) {
        return skill;
    }
    let Some(l) = w.units.get_mut(&key).and_then(|u| u.skills.as_mut()) else {
        return skill;
    };
    let Some(i) = l.find(0, super::skills::NATIVE) else {
        return skill;
    };
    if l.entries[i].param3 == 0 {
        l.entries[i].param3 = 5;
        5
    } else {
        l.entries[i].param3 = 0;
        0
    }
}

/// Step (a) of the client skill start:
/// when U's current entry is not of `skill`, it becomes the entry
/// (skill, owner); a list without that entry first gets the skill
/// and is looked up again; found after the add, the
/// local player's own (owner −1) entry gets flag 8 and the
/// request's level is split onto it. `false`: still no entry
/// (the start returns 0). A unit without a skill list: `true`, nothing
/// changes.
fn start_entry(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    skill: u16,
    owner: u32,
    level: i32,
) -> bool {
    use super::skills::{self, Owner, NATIVE};
    let local = w.local_player == Some(key);
    let Some(u) = w.units.get_mut(&key) else {
        return true;
    };
    let unit = Owner {
        unit_type: key.unit_type,
        class: u.class,
    };
    let Some(l) = u.skills.as_mut() else {
        return true;
    };
    if l.current
        .and_then(|i| l.entries.get(i))
        .is_some_and(|e| e.skill == skill)
    {
        return true;
    }
    let rows = &inputs.tables.skills;
    let found = match l.find(skill, owner) {
        Some(i) => Some(i),
        None => {
            // A refresh error (a passive skill) comes after the list is
            // updated: the entry stays.
            let _ = skills::add(l, rows, unit, skill);
            let i = l.find(skill, owner);
            if let Some(i) = i {
                if local && owner == NATIVE {
                    l.entries[i].flags |= 8;
                }
                let _ = skills::split_level(l, rows, unit, skill, level);
            }
            i
        }
    };
    match found {
        Some(i) => {
            l.current = Some(i);
            true
        }
        None => false,
    }
}
