//! What creates and changes light records.
//!
//! Each source is a pure function that answers with a [`LightRequest`]
//! (or a radius, or nothing). The caller applies it to the light list,
//! whose create clamps the radius to 1–18 and creates nothing for a radius
//! below 1.
//!
//! Every source light is full intensity; what differs is the kind, the
//! radius and the colour:
//! - players carry a white light of 13 sub-tiles, grown by
//!   `item_lightradius` and tinted by `item_lightcolor`;
//! - monsters light up by their `monstats2` row or their equipment;
//! - champions and uniques with the light modifier get a radius-7 light;
//! - overlays, skill casts and missiles carry plain lights that grow to
//!   their target, and missiles may flicker;
//! - objects carry cached lights by their mode's `Lit` column.

use d2_sim::rng::Seed;

/// The intensity of every source.
pub const SOURCE_INTENSITY: u8 = 255;
/// The player's base light radius.
pub const PLAYER_RADIUS: i32 = 13;
/// The light radius of the light modifier (umod 3).
pub const UMOD3_RADIUS: i32 = 7;

/// A light to create with an optional target to set right
/// after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LightRequest {
    /// Kind: 0 shadowed at the highest quality, 1 never shadowed, 2 cached
    /// shadow grid.
    pub kind: u8,
    /// Radius in sub-tiles, before the list clamps it.
    pub radius: i32,
    pub i: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// Target radius in sub-tiles, when the source sets one.
    pub target: Option<i32>,
}

impl LightRequest {
    fn new(kind: u8, radius: i32, (r, g, b): (u8, u8, u8)) -> Self {
        LightRequest {
            kind,
            radius,
            i: SOURCE_INTENSITY,
            r,
            g,
            b,
            target: None,
        }
    }
}

const WHITE: (u8, u8, u8) = (255, 255, 255);

/// A player's light: radius 13, white; shadowed for the local player (and
/// for any player before the local one exists), plain for everyone else.
pub fn player_light(local_or_no_local: bool) -> LightRequest {
    LightRequest::new(if local_or_no_local { 0 } else { 1 }, PLAYER_RADIUS, WHITE)
}

/// The player's light radius after stat 89 `item_lightradius` changed:
/// 13 + the new value, applied at once (no walk).
pub fn player_light_radius(item_lightradius: i32) -> i32 {
    PLAYER_RADIUS.wrapping_add(item_lightradius)
}

/// The player's light colour after stat 90 `item_lightcolor` changed: the
/// stat is `0xRRGGBB`, and 0 means plain white.
pub fn player_light_color(item_lightcolor: u32) -> (u8, u8, u8) {
    if item_lightcolor == 0 {
        return WHITE;
    }
    (
        (item_lightcolor >> 16) as u8,
        (item_lightcolor >> 8) as u8,
        item_lightcolor as u8,
    )
}

/// `L_c`, a monster's light from its equipment: the largest
/// `lightradius` among the items of its components (the caller skips the
/// codes none, `lit`, `med` and `hvy`); 0 when none.
pub fn component_light(radii: impl IntoIterator<Item = u8>) -> i32 {
    radii.into_iter().map(i32::from).max().unwrap_or(0)
}

/// Inputs of the monster light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MonsterLightInput {
    /// `L_c` ([`component_light`]).
    pub l_c: i32,
    /// `monstats2` `Light`.
    pub light: i32,
    /// `monstats2` `light-r`, `light-g`, `light-b`.
    pub rgb: (u8, u8, u8),
    /// The level id of the monster's room.
    pub level: u32,
    /// Client quest byte 1 ≠ 0.
    pub quest_byte1: bool,
    /// The monster is a client-only unit.
    pub client_only: bool,
    /// `monstats` `Align` (1 and 2: allies and neutrals).
    pub align: i32,
}

/// A monster's light: shadowed, radius `max(L_c, Light)` in the
/// `monstats2` colour. In the Den of Evil with its quest open, every
/// hostile server monster glows at radius 3 instead. `None` when the
/// radius is 0. It replaces the unit's previous light.
pub fn monster_light(m: &MonsterLightInput) -> Option<LightRequest> {
    let mut radius = m.l_c.max(m.light);
    if m.level == 8 && m.quest_byte1 && !m.client_only && !matches!(m.align, 1 | 2) {
        radius = 3;
    }
    (radius != 0).then(|| LightRequest::new(0, radius, m.rgb))
}

/// The light modifier (umod 3) with argument `u` (the unique type flag
/// 0x8). `u` = 0 → `None`: nothing happens and the old light stays.
/// Otherwise the unit's light is replaced by a shadowed radius-7 light. A
/// champion (type flag 0x4) gets a random colour: three steps of its seed,
/// R, G, B = the low bytes of the third, second and first new low words.
/// Any other monster takes its `monstats2` colour.
pub fn umod3_light(
    u: u32,
    champion: bool,
    seed: &mut Seed,
    monstats2_rgb: (u8, u8, u8),
) -> Option<LightRequest> {
    if u == 0 {
        return None;
    }
    let rgb = if champion {
        let first = seed.step();
        let second = seed.step();
        let third = seed.step();
        (third as u8, second as u8, first as u8)
    } else {
        monstats2_rgb
    };
    Some(LightRequest::new(0, UMOD3_RADIUS, rgb))
}

/// An overlay's light: plain, radius `InitRadius` growing to `Radius` when
/// they differ; `None` when `Radius` is 0.
pub fn overlay_light(init_radius: i32, radius: i32, rgb: (u8, u8, u8)) -> Option<LightRequest> {
    if radius == 0 {
        return None;
    }
    let mut req = LightRequest::new(1, init_radius, rgb);
    if radius != init_radius {
        req.target = Some(radius);
    }
    Some(req)
}

/// The creation flag that suppresses a missile's light.
pub const MISSILE_NO_LIGHT_FLAG: u32 = 0x4000;

/// A missile's light when the client creates it: only with missile lights
/// on (high quality), without the no-light creation flag, and when the
/// missile row's `Light` or the creation record's light-request byte is
/// set. Plain, radius `Light` always: with `Light` = 0 the create gets
/// radius 0, which creates nothing, so the request only removes the old
/// light.
pub fn missile_light(
    missile_lights: bool,
    creation_flags: u32,
    light: i32,
    create_0x50: u8,
    rgb: (u8, u8, u8),
) -> Option<LightRequest> {
    if !missile_lights || creation_flags & MISSILE_NO_LIGHT_FLAG != 0 {
        return None;
    }
    if light == 0 && create_0x50 == 0 {
        return None;
    }
    Some(LightRequest::new(1, light, rgb))
}

/// Missile flicker: with missiles `Flicker` ≠ 0, the
/// missile's frame `+0x44` (8.8) & 0x300 = 0, a light, and its radius (in
/// sub-tiles) ≥ `Light`: the new target `Light` + `roll(Flicker)` from the
/// missile unit's seed. `None` otherwise (no step).
pub fn missile_flicker(
    frame: u32,
    radius: Option<i32>,
    light: i32,
    flicker: i32,
    seed: &mut Seed,
) -> Option<i32> {
    if flicker == 0 || frame & 0x300 != 0 {
        return None;
    }
    let radius = radius?;
    (radius >= light).then(|| light.wrapping_add(seed.roll(flicker) as i32))
}

/// A skill cast's light from its `cltmissile`'s `Light` and colour: plain,
/// radius 1 growing to `Light`; `None` when `Light` is 0. It replaces the
/// unit's light.
pub fn skill_cast_light(light: i32, rgb: (u8, u8, u8)) -> Option<LightRequest> {
    if light == 0 {
        return None;
    }
    let mut req = LightRequest::new(1, 1, rgb);
    req.target = Some(light);
    Some(req)
}

/// What an object's mode change does to its light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectLight {
    /// `Lit` = 0: the light is removed.
    Remove,
    /// No light yet: create one.
    Create(LightRequest),
    /// The object has a light: walk it to this radius (sub-tiles).
    SetTarget(i32),
}

/// An object's light for its mode's `Lit` value (objects are created with
/// `Lit2`): cached, radius `Lit` / 2, in the `objects` colour.
pub fn object_light(lit: u8, has_light: bool, rgb: (u8, u8, u8)) -> ObjectLight {
    if lit == 0 {
        return ObjectLight::Remove;
    }
    let radius = i32::from(lit) / 2;
    if has_light {
        ObjectLight::SetTarget(radius)
    } else {
        ObjectLight::Create(LightRequest::new(2, radius, rgb))
    }
}

/// The light of overlay 182 `horadric_light`: cached, white, radius 60
/// (the list clamps it to 18).
pub fn horadric_light() -> LightRequest {
    LightRequest::new(2, 60, WHITE)
}

/// The light of missile 191 `cursecenter`: plain, red, radius 1 growing to
/// the curse's area (`aurarangecalc`, at least 2).
pub fn cursecenter_light(aurarangecalc: i32) -> LightRequest {
    let mut req = LightRequest::new(1, 1, (255, 0, 0));
    req.target = Some(aurarangecalc.max(2));
    req
}
