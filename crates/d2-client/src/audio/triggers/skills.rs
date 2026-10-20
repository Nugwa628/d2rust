//! Sound columns of `skills`, `missiles`, `states` and the item tables.
//! Inputs are the columns as signed plain values (the rules test `> 0` /
//! `≥ 0`); the callers read them from the typed tables.

use super::{sid, swing, swing_entry, Ctx, TriggerError, Unit, MONSTER, PLAYER};
use crate::bridge::world::UnitKey;

/// The `skills` columns the skill start sounds read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkillStart {
    /// `stsound`.
    pub stsound: i32,
    /// `stsoundclass`.
    pub stsoundclass: i32,
    /// `stsounddelay` flag.
    pub stsounddelay: bool,
    /// `weaponsnd` flag.
    pub weaponsnd: bool,
    /// `stsuccessonly` flag.
    pub stsuccessonly: bool,
    /// `charclass` (+0x0C).
    pub charclass: i32,
    /// `ItemCastSound`, when cast from an item (item ≠ −1); `None` otherwise.
    pub item_cast_sound: Option<i32>,
}

/// Skill start sounds, after the start function
/// (`start_ok` = it returned non-zero) and the mode set.
pub fn skill_start(
    cx: &mut Ctx,
    u: &Unit,
    sk: &SkillStart,
    start_ok: bool,
) -> Result<(), TriggerError> {
    if !start_ok && sk.stsuccessonly {
        return Ok(());
    }
    let h = u.weapon_hit_class;
    // Delay of the start sound and the class sound: swing − 3, signed, passed as is.
    let delay = || -> Result<u32, TriggerError> {
        Ok(if sk.stsounddelay {
            (swing(u.speed, h)? as i32).wrapping_sub(3) as u32
        } else {
            0
        })
    };
    // The start sound (an item's cast sound replaces it).
    let mut s = sk.stsound;
    let replaced = matches!(sk.item_cast_sound, Some(x) if x > 0);
    if let (true, Some(x)) = (replaced, sk.item_cast_sound) {
        s = x;
    }
    if s > 0 {
        let d = delay()?;
        cx.req(s, Some(u.key), d);
    }
    // The weapon swing.
    if sk.weaponsnd {
        let (id, _) = swing_entry(h)?;
        if id != 0 {
            let d = swing(u.speed, h)?;
            cx.req(id, Some(u.key), d);
        }
    }
    // The class start sound.
    if sk.stsoundclass > 0 && !replaced && u.unit_type() == PLAYER && u.class == sk.charclass {
        let d = delay()?;
        cx.req(sk.stsoundclass, Some(u.key), d);
    }
    Ok(())
}

/// Skill do / target sounds, after the client do function
/// returned non-zero: `dosound` > 0 on the caster, `tgtsound` > 0 on the
/// target if any. `dosound a` / `dosound b` are [`jab_do`] and
/// [`charge_start`].
pub fn skill_do(
    cx: &mut Ctx,
    caster: UnitKey,
    target: Option<UnitKey>,
    dosound: i32,
    tgtsound: i32,
) {
    if dosound > 0 {
        cx.req(dosound, Some(caster), 0);
    }
    if let (true, Some(t)) = (tgtsound > 0, target) {
        cx.req(tgtsound, Some(t), 0);
    }
}

/// `cltdofunc` 16 (live: Jab): when the
/// caster's action frame event (U +0x4E) is 3, a player caster requests
/// `dosound a` (+0x102) and a monster caster `dosound b` (+0x104), on the
/// caster, delay 0, when > 0. Returns 1 either way.
pub fn jab_do(cx: &mut Ctx, caster: &Unit, frame_event: u8, dosound_a: i32, dosound_b: i32) -> i32 {
    if frame_event == 3 {
        let id = match caster.unit_type() {
            PLAYER => dosound_a,
            MONSTER => dosound_b,
            _ => 0,
        };
        if id > 0 {
            cx.req(id, Some(caster.key), 0);
        }
    }
    1
}

/// `cltstfunc` 25 (live: Charge, SerpentCharge), when it does not hand
/// over to the charge start and its
/// entry checks pass (the caller's): a player requests `dosound a` (> 0);
/// a monster the `monsounds` skill voice of the monstats skill slot
/// holding this skill (slots 0–3 → `Skill1`–`Skill4`;
/// another slot or none → nothing); on the caster, delay 0.
pub fn charge_start(cx: &mut Ctx, caster: &Unit, dosound_a: i32, skill_slot: Option<usize>) {
    let id = match caster.unit_type() {
        PLAYER => dosound_a,
        MONSTER => match (skill_slot, caster.monsounds) {
            (Some(slot @ 0..=3), Some(r)) => sid([r.skill1, r.skill2, r.skill3, r.skill4][slot]),
            _ => 0,
        },
        _ => 0,
    };
    if id > 0 {
        cx.req(id, Some(caster.key), 0);
    }
}

/// Item cast sound `ItemCastSound` ≥ 0 on the caster.
pub fn item_cast(cx: &mut Ctx, caster: UnitKey, item_cast_sound: i32) {
    if item_cast_sound >= 0 {
        cx.req(item_cast_sound, Some(caster), 0);
    }
}

/// `prgsound` on the unit from the progressive function.
/// The caller decides when.
pub fn skill_progressive(cx: &mut Ctx, unit: UnitKey, prgsound: i32) {
    cx.req(prgsound, Some(unit), 0);
}

/// Missile `TravelSound` after its client init.
pub fn missile_travel(cx: &mut Ctx, missile: UnitKey, travel_sound: i32) {
    cx.req(travel_sound, Some(missile), 0);
}

/// Missile `HitSound` ≥ 0 when its client hit function returned non-zero.
pub fn missile_hit(cx: &mut Ctx, missile: UnitKey, hit_sound: i32) {
    if hit_sound >= 0 {
        cx.req(hit_sound, Some(missile), 0);
    }
}

/// Missile `ProgSound` of client missile function 9 (its
/// `CltSubMissile1` / skill-row precondition is
/// the caller's): at elapsed = max(P1, 1) − 2 the missile's first attached
/// request (its list head) is detached with force, then
/// `ProgSound` > 0 is requested on the missile, delay 0.
pub fn missile_prog_fn9(cx: &mut Ctx, missile: UnitKey, elapsed: i32, p1: i32, prog_sound: i32) {
    if elapsed != p1.max(1) - 2 {
        return;
    }
    if let Some(&(h, _)) = cx.s.unit_requests(missile).first() {
        cx.s.detach(h, missile, true);
    }
    if prog_sound > 0 {
        cx.req(prog_sound, Some(missile), 0);
    }
}

/// Function 29:
/// elapsed > 10, the sub missile S = `CltSubMissile1` > 0 and elapsed =
/// 315 → S's `ProgSound` > 0 on the missile's **owner**, delay 0.
pub fn missile_prog_fn29(
    cx: &mut Ctx,
    owner: Option<UnitKey>,
    elapsed: i32,
    sub_missile: i32,
    sub_prog_sound: i32,
) {
    if elapsed > 10 && sub_missile > 0 && elapsed == 315 && sub_prog_sound > 0 {
        cx.req(sub_prog_sound, owner, 0);
    }
}

/// Function 47: with `ProgSound` > 0,
/// a = missile data +0x28 and b = its client motion record +0x40: a ≠ 0
/// and a ≠ b → request on the missile, delay 0; then data +0x28 := b.
///
/// The update of data +0x28 sits inside the `ProgSound` > 0 branch:
/// `ProgSound` ≤ 0 skips it too. The live missile 452 has `ProgSound`
/// 2,416, so the data cannot show the other
/// reading.
pub fn missile_prog_fn47(
    cx: &mut Ctx,
    missile: UnitKey,
    prog_sound: i32,
    data_28: &mut i32,
    b: i32,
) {
    if prog_sound <= 0 {
        return;
    }
    if *data_28 != 0 && *data_28 != b {
        cx.req(prog_sound, Some(missile), 0);
    }
    *data_28 = b;
}

/// Function 51: the client missile
/// `CltSubMissile2` created at elapsed = P2 (the caller's); created and
/// `ProgSound` > 0 → request on the **new** missile, delay 0.
pub fn missile_prog_fn51(cx: &mut Ctx, created: Option<UnitKey>, prog_sound: i32) {
    if let (Some(m), true) = (created, prog_sound > 0) {
        cx.req(prog_sound, Some(m), 0);
    }
}

/// State on (S→C 0xA7/0xA8/0xAA). `had` = U had the
/// state; `dead` = U is a monster in mode 12 or a player in mode 17.
pub fn state_on(cx: &mut Ctx, unit: UnitKey, had: bool, notondead: bool, dead: bool, onsound: i32) {
    if notondead && dead {
        return;
    }
    if !had && onsound >= 0 {
        cx.req(onsound, Some(unit), 0);
    }
}

/// State off (S→C 0xA9).
pub fn state_off(cx: &mut Ctx, unit: UnitKey, had: bool, offsound: i32) {
    if had && offsound >= 0 {
        cx.req(offsound, Some(unit), 0);
    }
}

/// An item row's sound columns (`dropsound`, `dropsfxframe`, `usesound`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemSounds {
    pub dropsound: i32,
    pub dropsfxframe: u32,
    pub usesound: i32,
}

/// The rows an item's sounds come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemRows {
    /// Base row (`weapons` / `armor` / `misc`).
    pub base: ItemSounds,
    /// Item quality (5 set, 7 unique).
    pub quality: u8,
    /// The unique row (quality 7) or set row (quality 5), if any.
    pub special: Option<ItemSounds>,
}

/// Default drop frame and the drop volume.
pub const DROP_FRAME_DEFAULT: u32 = 12;
pub const DROP_VOLUME: i32 = 180;
/// Fixed item sounds.
pub const ITEM_PICKUP: i32 = 235;
pub const ITEM_FLIPPY: i32 = 216;
pub const ITEM_GOLD: i32 = 221;

impl ItemRows {
    fn special_row(&self) -> Option<&ItemSounds> {
        match self.quality {
            5 | 7 => self.special.as_ref().filter(|r| r.dropsound > 0),
            _ => None,
        }
    }

    /// Drop sound and delay.
    pub fn drop_sound(&self) -> (i32, u32) {
        let mut id = self.base.dropsound;
        let mut d = match self.base.dropsfxframe {
            0 => DROP_FRAME_DEFAULT,
            f => f,
        };
        if let Some(r) = self.special_row() {
            id = r.dropsound;
            if r.dropsfxframe != 0 {
                d = r.dropsfxframe;
            }
        }
        (id, d)
    }

    /// Place sound: the drop sound without frame.
    pub fn place_sound(&self) -> i32 {
        self.drop_sound().0
    }

    /// Use sound (the `dropsound` test is kept).
    pub fn use_sound(&self) -> i32 {
        match self.special_row() {
            Some(r) => r.usesound,
            None if self.base.dropsound > 0 => self.base.usesound,
            None => 0,
        }
    }
}

/// Item mode 4, on cursor.
pub fn item_to_cursor(cx: &mut Ctx) {
    cx.req(ITEM_PICKUP, None, 0);
}

/// Item mode 5, dropping to the ground.
pub fn item_drop(cx: &mut Ctx, item: UnitKey, rows: &ItemRows) {
    cx.req(ITEM_FLIPPY, Some(item), 0);
    let (id, d) = rows.drop_sound();
    if id > 0 {
        let h = cx.req(id, Some(item), d);
        if h != 0 {
            cx.s.set_volume(h, DROP_VOLUME);
        }
    }
}

/// Item placed in a grid: request(place sound, none).
pub fn item_place(cx: &mut Ctx, rows: &ItemRows) {
    cx.req(rows.place_sound(), None, 0);
}

/// Item used: request(use sound, none).
pub fn item_use(cx: &mut Ctx, rows: &ItemRows) {
    cx.req(rows.use_sound(), None, 0);
}

/// Gold: 221 `item_gold`, none.
pub fn item_gold(cx: &mut Ctx) {
    cx.req(ITEM_GOLD, None, 0);
}

// ---------------------------------------------------------------------------
// Animation event 3 ("sound")
// ---------------------------------------------------------------------------

/// AnimData event bytes: 1 attack, 2 missile, 3 sound,
/// 4 skill.
pub const EVENT_SOUND: u8 = 3;

/// The frame advance clears U +0x4E, then for each
/// frame it crosses stores the event byte of that frame if it is 1–4, so
/// the stored value is the last such event, or 0.
pub fn frame_event_after_advance(crossed: &[u8]) -> u8 {
    crossed
        .iter()
        .copied()
        .rfind(|e| (1..=4).contains(e))
        .unwrap_or(0)
}

/// Player update: the generic skill do runs in a mode whose movement
/// entry is 2 with a current
/// skill, unless the skill's flag bit 0 path already ran the do this
/// update, with U flag +0xC4 bit 0x40 clear and +0x4E ∈ {1, 2, 3}.
pub fn player_generic_do(
    movement_entry: u8,
    has_skill: bool,
    flag0_path_ran: bool,
    flag_c4_40_set: bool,
    event: u8,
) -> bool {
    movement_entry == 2 && has_skill && !flag0_path_ran && !flag_c4_40_set && matches!(event, 1..=3)
}

/// Monster update: +0x4E = 4 runs the do; 1, 2 or 3 run it unless it
/// already ran or the unit's flag 0x40 is set.
pub fn monster_generic_do(event: u8, already_ran: bool, flag_40_set: bool) -> bool {
    match event {
        4 => true,
        1..=3 => !already_ran && !flag_40_set,
        _ => false,
    }
}

/// `cltdofunc` 37 (Charge, SerpentCharge): a player or monster with
/// +0x4E = 3 plays its attack sound (U, U's
/// mode, 0)` with delay argument 0.
pub fn event3_attack(
    cx: &mut Ctx,
    u: &Unit,
    us: &mut super::UnitSound,
    event: u8,
) -> Result<(), TriggerError> {
    if event != EVENT_SOUND {
        return Ok(());
    }
    match u.identity() {
        PLAYER => super::modes::player_attack_with(cx, u, us, false),
        MONSTER => {
            if let Some(r) = u.monsounds {
                let slot = if u.mode == super::modes::mm::A1 { 1 } else { 2 };
                super::modes::monster_attack(cx, u, us, r, slot);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
