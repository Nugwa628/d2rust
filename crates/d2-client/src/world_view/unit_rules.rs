//! [`UnitRules`]: the unit hooks of [`ViewRules`] answered through
//! `rules::unit_composite` from the unit art the [`UnitArtLoader`] made
//! resident. Every other hook (tiles, UI) is the wrapped rules'.
//!
//! The play preview ("First playable preview") fills
//! what the model lacks, each fill marked `D2RUST's own, unverified`:
//! - direction: the model holds no client path record; `dir64` is the
//!   unit art's preview facing (`UnitArt::dir64`: the predicted facing of
//!   the local player, else the facing toward the walk target or the last
//!   position change, else 0), mapped to the COF row with the
//!   expected count (`UnitArt::expected_directions`);
//! - frame: objects take the model's +0x44 frame as is (the
//!   client object update animates it); players, monsters and missiles
//!   the COF's animation rate (per base-rate tick) advanced by the server
//!   tick count at the game's rate (the client model runs no animation
//!   for them yet);
//! - the COF box pre-test is the view's (`OriginalView`, which
//!   knows the final screen position);
//! - draw key: pass 6 at major / minor 0 when the source states no draw
//!   order (`OriginalView` overwrites it when it does);
//! - shade: none (full bright, `v = 0xFF`); blend:
//!   opaque (COF layer translucency not applied).
//!
//! [`UnitArtLoader`]: super::unit_assets::UnitArtLoader

pub mod animation_bake;
pub mod animation_baked;
pub mod animation_build;
pub mod animation_deny;
pub mod animation_inputs;
pub mod animation_pack;
pub mod animation_proof;
pub mod frame_pack;
pub mod frame_source;
pub mod generation_job;
pub mod generator;
pub mod playback;

use crate::rules::unit_composite::CofPath;
use frame_source::{FrameRequest, FrameSource, SourceFrames};

use std::sync::Arc;

use crate::bridge::world::{ClientWorld, KindData, MONSTER, OBJECT, PLAYER};
use crate::bridge::ClientUnit;
use d2_formats::cof::Cof;
use d2_sim::tick::TickRate;

use crate::composite::{
    ComponentDraw, ComponentFrame, ComponentRequest, CompositeError, UnitParams,
};
use crate::frames::IndexFrame;
use crate::rules::draw_order::OrderKey;
use crate::rules::unit_composite::{
    component_cel, file_format, frame_index, unit_direction, CompositeKind,
};
use crate::scene::{order::pass, BlendOp, DrawItem, ItemTag, Rect, ShadeChain};
use crate::ui::{ImageRequest, TextRequest};

use super::unit_assets::{component_codes, unit_cof_with, SharedUnitArt, UnitLooks};
use super::{SlotCall, TileDraw, UiRules, UiSprite, UnitPose, ViewAssets, ViewError, ViewRules};

/// `rules` with the unit composite hooks of the play preview.
pub struct UnitRules<R, F = SourceFrames> {
    /// Selects component pictures without changing the simulation or COF order.
    pub frame_source: F,
    pub rules: R,
    pub looks: Arc<UnitLooks>,
    pub art: SharedUnitArt,
}

impl<R, F> UnitRules<R, F> {
    /// The COF frame of `unit` this tick (module doc: D2RUST's own,
    /// unverified).
    fn frame(world: &ClientWorld, unit: &ClientUnit, frames: u8, rate: u32) -> usize {
        // +0x44 >> 8 as is (no bound) where the model
        // animates the unit: objects (the client object update,
        // generic step).
        if unit.key.unit_type == OBJECT {
            return frame_index(unit.frame as u32);
        }
        // Monsters: the model's own frame, advanced from
        // the mode set's by `bridge::monster_anim`; with a sequence, the
        // frame its setup drew.
        // Never the player's tick clock, also
        // with frame count 0 (every base-game monster mode has an AnimData
        // record: 11744 of 11744 class × mode, `client_monster_anims`).
        if unit.key.unit_type == MONSTER {
            return frame_index(unit.frame as u32);
        }
        // A player in a neutral mode whose mode set the model ran
        // (`player_anim::mode_set`, e.g. the 0x15 placement out of town):
        // the model's own looping frame.
        if unit.key.unit_type == PLAYER
            && (matches!(unit.mode, 1 | 5)
                || crate::bridge::player_anim::ENDING_MODES.contains(&unit.mode))
            && unit.speed.is_some()
            && unit.frame_count > 0
        {
            return frame_index(unit.frame as u32);
        }
        // (measured for players; for missiles settled by a recorded Frost
        // Nova, whose list holds the nova missiles and matches): the draw
        // of tick T has had T − 1 advances of the
        // 8.8 rate, stated per base-rate tick.
        tick_frame(world.server_ticks, frames, rate, world.tick_rate)
    }
}

/// revision (settled: unit flag 0x20): object
/// `class` in `mode` casts its composite shadow when its `BlocksLight` of
/// that mode is ≠ 0; a class without a row casts it.
/// The mode whose `BlocksLight` the object's flag 0x20 holds: the one
/// of its last set-up or 0x0E mode change (the mode-1 finish keeps it),
/// else the current mode.
pub fn object_shadow_mode(unit: &ClientUnit) -> u32 {
    match &unit.kind {
        KindData::Object(d) => d.shadow_mode.unwrap_or(unit.mode),
        _ => unit.mode,
    }
}

pub fn object_casts_shadow(looks: &UnitLooks, class: u32, mode: u32) -> bool {
    looks.object_blocks_light.get(&class).is_none_or(|b| {
        usize::try_from(mode)
            .ok()
            .and_then(|m| b.get(m))
            .is_some_and(|&v| v != 0)
    })
}

/// The player's walk speed (w = 213 for a
/// player not running, p = 100 without item / skill velocity).
pub const PLAYER_WALK_SPEED: u64 = 213;

/// The player's run speed (w = 101, p = the
/// run's velocity percent 100 · `RunVelocity` / `WalkVelocity`; 150 for
/// the sorceress' 9 / 6: speed 151, `a1-run-*`). Without the charstats
/// speeds p = 100.
pub fn player_run_speed(speeds: Option<crate::bridge::predict::Speeds>) -> u64 {
    let p = speeds
        .filter(|s| s.walk > 0)
        .map_or(100, |s| 100 * u64::from(s.run) / u64::from(s.walk));
    101 * p / 100
}

/// The walk frame (measured on walks and runs; the run's speed by the
/// unit velocity's rate rule): the frame at server tick `tick` of
/// a walk or run whose mode started on `since` (kept across re-targeting
/// clicks, restarted by a walk ↔ run change): `(progress >> 8) mod
/// frames`, where the progress is `speed` (8.8 frames per base-rate tick)
/// over `tick − since` ticks of the game's rate `at`
/// ([`TickRate::progress`]), so a walk cycle lasts the same real time at
/// every rate.
pub fn walk_frame(tick: u64, since: u64, frames: u8, speed: u64, at: TickRate) -> usize {
    let frames = u64::from(frames.max(1));
    ((at.progress(speed, tick.saturating_sub(since)) >> 8) % frames) as usize
}

/// The frame drawn at server tick `tick`: the 8.8 `rate` (per base-rate
/// tick) over `tick − 1` ticks of the game's rate `at`, `(progress >> 8)
/// mod frames`.
pub fn tick_frame(tick: u64, frames: u8, rate: u32, at: TickRate) -> usize {
    let frames = u64::from(frames.max(1));
    ((at.progress(u64::from(rate), tick.saturating_sub(1)) >> 8) % frames) as usize
}

fn unresolved(req: &ComponentRequest<'_>, what: &'static str, message: String) -> CompositeError {
    CompositeError::Unresolved {
        slot: req.slot.slot,
        component: req.slot.component,
        what,
        message,
    }
}

impl<R: ViewRules, F: FrameSource> ViewRules for UnitRules<R, F> {
    fn tiles(&self, world: &ClientWorld, assets: &ViewAssets) -> Result<Vec<TileDraw>, ViewError> {
        self.rules.tiles(world, assets)
    }

    /// `None` unless the unit has a COF the loader made resident.
    fn unit_pose(
        &self,
        world: &ClientWorld,
        unit: &ClientUnit,
    ) -> Result<Option<UnitPose>, ViewError> {
        let art = self.art.read().unwrap_or_else(|e| e.into_inner());
        let unit = &*art.posed(unit);
        let Some(name) = unit_cof_with(&self.looks, unit, art.hands_of(unit.key)) else {
            return Ok(None);
        };
        let Ok(path) = name.path() else {
            return Ok(None);
        };
        let Some(cof) = art.cofs.get(&path) else {
            return Ok(None);
        };
        let dead = name.kind != CompositeKind::Object && unit.is_dead();
        let n = art.expected_directions(&self.looks, unit, name.kind, cof.directions);
        let Ok(dir) = unit_direction(cof.directions, n, art.dir64(unit), dead) else {
            return Ok(None);
        };
        let sequence = art.sequence.filter(|(key, _)| *key == unit.key);
        let walking = art
            .pose_since
            .filter(|(key, ..)| *key == unit.key && matches!(unit.mode, 2 | 3 | 6));
        let sub_frame = if sequence.is_some() {
            // Sequence setup supplies only a drawn source frame.
            0
        } else if let Some((_, since, speed)) = walking {
            world
                .tick_rate
                .progress(speed, world.server_ticks.saturating_sub(since)) as u8
        } else if unit.key.unit_type == OBJECT
            || unit.key.unit_type == MONSTER
            || (unit.key.unit_type == PLAYER
                && (matches!(unit.mode, 1 | 5)
                    || crate::bridge::player_anim::ENDING_MODES.contains(&unit.mode))
                && unit.speed.is_some()
                && unit.frame_count > 0)
        {
            unit.frame as u8
        } else {
            world.tick_rate.progress(
                u64::from(cof.animation_rate),
                world.server_ticks.saturating_sub(1),
            ) as u8
        };
        Ok(Some(UnitPose {
            sub_frame,
            cof: path,
            dir: usize::from(dir.cof_dir),
            dir64: dir.dir64,
            frame: if let Some((_, f)) = art.sequence.filter(|(k, _)| *k == unit.key) {
                // The sequence frame's drawn frame.
                f % usize::from(cof.frames.max(1))
            } else if let Some((since, speed)) = art
                .pose_since
                .filter(|(k, ..)| *k == unit.key && matches!(unit.mode, 2 | 3 | 6))
                .map(|(_, s, v)| (s, v))
            {
                walk_frame(
                    world.server_ticks,
                    since,
                    cof.frames,
                    speed,
                    world.tick_rate,
                )
            } else {
                Self::frame(world, unit, cof.frames, cof.animation_rate)
            },
        }))
    }

    fn unit_params(
        &self,
        _: &ClientWorld,
        unit: &ClientUnit,
        _: &UnitPose,
    ) -> Result<UnitParams, ViewError> {
        // D2RUST's own, unverified (the preview): `OriginalView` sets the draw key
        // from the draw order when its source has one, and the clip.
        Ok(UnitParams {
            pass: pass::WALLS_UNITS,
            major: 0,
            minor: 0,
            clip: Rect::world_clip(),
            tag: ItemTag::Unit(unit.key.guid),
        })
    }

    /// The body is skipped for an object whose
    /// `Draw` is 0 (the call alone); its shadow
    /// still draws (measured: object 385 `cain start position`).
    fn unit_draws_body(&self, unit: &ClientUnit) -> bool {
        !(unit.key.unit_type == OBJECT && self.looks.object_no_draw.contains(&unit.class))
    }

    fn unit_light_word(&self, unit: &ClientUnit) -> Option<u32> {
        self.rules.unit_light_word(unit)
    }

    fn unit_client_point(&self, unit: &ClientUnit) -> Option<(i32, i32)> {
        self.rules.unit_client_point(unit)
    }

    fn component_call(
        &self,
        unit: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Option<crate::scene::CelArgs> {
        self.rules.component_call(unit, req)
    }

    /// The shadows of the composite just built (`unit_shadow`).
    fn unit_shadows(
        &self,
        _world: &ClientWorld,
        unit: &ClientUnit,
        pose: &UnitPose,
        at: Option<OrderKey>,
        draws: &[ComponentDraw],
        assets: &ViewAssets,
    ) -> Result<Vec<DrawItem>, ViewError> {
        let (Some(at), Some(cof)) = (at, assets.cofs.get(&pose.cof)) else {
            return Ok(Vec::new());
        };
        // revision (settled): an
        // object casts the shadow only when BlocksLight ≠ 0 in the mode
        // that last wrote flag 0x20 (init or 0x0E change; the mode-1
        // finish keeps it).
        if unit.key.unit_type == OBJECT
            && !object_casts_shadow(&self.looks, unit.class, object_shadow_mode(unit))
        {
            return Ok(Vec::new());
        }
        super::unit_shadow::draws(cof, unit.key.guid, at, draws, assets)
    }

    fn component_frame(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        req: &ComponentRequest<'_>,
    ) -> Result<ComponentFrame, CompositeError> {
        self.component_slot_frame(unit, pose, req)?
            .ok_or_else(|| unresolved(req, "component frame", "the slot draws nothing".into()))
    }

    /// The slots whose request succeeds but whose file is in no
    /// archive (the loader's `None` entry); the base game still calls the drawer.
    fn unit_slot_calls(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        cof: &Cof,
    ) -> Result<Vec<SlotCall>, ViewError> {
        let art = self.art.read().unwrap_or_else(|e| e.into_inner());
        let unit = &*art.posed(unit);
        let Some(name) = unit_cof_with(&self.looks, unit, art.hands_of(unit.key)) else {
            return Ok(Vec::new());
        };
        let slots = crate::composite::slot_order(cof, pose.dir, pose.frame).map_err(|error| {
            ViewError::Unit {
                unit_type: unit.key.unit_type,
                guid: unit.key.guid,
                error,
            }
        })?;
        let mut calls = Vec::new();
        for slot in slots {
            let Some(codes) = component_codes(
                &self.looks,
                unit,
                &name,
                &cof.layers[slot.layer],
                art.equip_of(unit.key),
            ) else {
                continue;
            };
            if let Some(None) = art.files.get(&codes.name()) {
                let format = file_format(&codes, unit.class, unit.mode as u8);
                if let Ok(path) = codes.path(format) {
                    calls.push(SlotCall {
                        slot: slot.slot,
                        layer: slot.layer,
                        path,
                    });
                }
            }
        }
        Ok(calls)
    }

    /// The item record whose code is the
    /// component's armor-class token, e.g.
    /// a monster's `tch` layer is the misc Torch (`transparent` 1,
    /// `transtbl` 3).
    fn component_item_trans(
        &self,
        unit: &ClientUnit,
        req: &ComponentRequest<'_>,
    ) -> Option<(u8, u8)> {
        let art = self.art.read().unwrap_or_else(|e| e.into_inner());
        let unit = &*art.posed(unit);
        let name = unit_cof_with(&self.looks, unit, art.hands_of(unit.key))?;
        let codes = component_codes(&self.looks, unit, &name, req.layer, art.equip_of(unit.key))?;
        self.looks.items.get(&codes.armor_class).map(|l| l.trans)
    }

    /// A failed request or a file that did not load draws
    /// nothing.
    fn component_slot_frame(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        req: &ComponentRequest<'_>,
    ) -> Result<Option<ComponentFrame>, CompositeError> {
        let art = self.art.read().unwrap_or_else(|e| e.into_inner());
        let unit = &*art.posed(unit);
        let Some(name) = unit_cof_with(&self.looks, unit, art.hands_of(unit.key)) else {
            return Ok(None);
        };
        let Some(codes) =
            component_codes(&self.looks, unit, &name, req.layer, art.equip_of(unit.key))
        else {
            return Ok(None);
        };
        let Some(Some((path, facts))) = art.files.get(&codes.name()) else {
            return Ok(None);
        };
        let n = art.expected_directions(&self.looks, unit, name.kind, req.cof.directions);
        let facing = art.dir64(unit).min(63);
        let dir = unit_direction(req.cof.directions, n, facing, false)
            .map_err(|e| unresolved(req, "direction", e.to_string()))?;
        // A block not loaded yet draws a loaded direction.
        let dir64 = art.block_dir64(path, name.kind, *facts, dir.dir64);
        // D2RUST's own, unverified (the preview): a cel past the file is skipped, not
        // a frame error.
        Ok(
            component_cel(path, facts.directions, facts.frames, dir64, req.frame)
                .ok()
                .map(|source| {
                    self.frame_source.frame(
                        &FrameRequest {
                            unit,
                            codes: &codes,
                            component: req.slot.component,
                            // Pack keys keep the unsnapped 64-space facing;
                            // the source cel still uses the snapped direction.
                            direction: facing,
                            source_direction: animation_bake::source_facing(
                                facts.directions,
                                dir64,
                            ),
                            frame: req.frame,
                            sub_frame: pose.sub_frame,
                        },
                        source,
                    )
                }),
        )
    }

    fn place(
        &self,
        unit: &ClientUnit,
        pose: &UnitPose,
        req: &ComponentRequest<'_>,
        image: &IndexFrame,
    ) -> Result<(i32, i32), CompositeError> {
        self.rules.place(unit, pose, req, image)
    }

    /// D2RUST's own, unverified (the preview): full bright, no remap.
    fn shade(
        &self,
        _: &ClientUnit,
        _: &ComponentRequest<'_>,
    ) -> Result<ShadeChain, CompositeError> {
        Ok(ShadeChain::EMPTY)
    }

    /// D2RUST's own, unverified (the preview): opaque.
    fn blend(&self, _: &ClientUnit, _: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
        Ok(BlendOp::Opaque)
    }
}

impl<R: UiRules, F> UiRules for UnitRules<R, F> {
    fn ui_image(&self, req: &ImageRequest, assets: &ViewAssets) -> Result<UiSprite, ViewError> {
        self.rules.ui_image(req, assets)
    }

    fn ui_text(&self, req: &TextRequest, assets: &ViewAssets) -> Result<Vec<UiSprite>, ViewError> {
        self.rules.ui_text(req, assets)
    }

    fn ui_pass(&self) -> Result<u32, ViewError> {
        self.rules.ui_pass()
    }
}

#[cfg(test)]
mod tests;
