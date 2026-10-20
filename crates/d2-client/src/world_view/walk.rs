//! The play preview's own-walk motion in the app: [`PreviewWalk`] holds
//! the local player's
//! [`Predict`], the run / stand-still modifiers ([`RunMods`]) and the
//! charstats [`Speeds`], and [`preview_walk_frame`] runs it after each
//! bridge frame:
//!
//! - the walks the [`crate::bridge::predict::PredictLink`] recorded since
//!   the last frame ([`WalkTap`]) set the target, and a server tick steps
//!   the prediction (`Predict::frame`);
//! - the predicted position goes to the feed (camera centre and the local
//!   player's draw, `ViewFeed::set_local_prediction`) and the predicted
//!   mode (2 walk / 3 run) and facing to the unit art
//!   (`UnitArt::pose_mode`, `UnitArt::pose_dir`).
//!
//! The world view reads [`PreviewWalk`] for its clicks: the `mods` word
//! (`RunMods::word`) and the predicted position as the click's local
//! player (`Bridge::world_click_at`).
//!
//! D2RUST's own plumbing (the base-game client's player stands at the server
//! player's point at every tick, "Measured
//! 2026-10-09", settled by that measurement). Only `play` inserts
//! it; the strict path never has one.

use bevy::prelude::*;

use crate::bridge::click::RunMods;

use crate::bridge::predict::{Predict, Speeds, WalkTap};
use crate::bridge::world::ClientWorld;
use crate::bridge::BridgeResource;

use super::unit_assets::SharedUnitArt;
use super::WorldViewState;

/// The local player's walk prediction of the play preview (module doc).
#[derive(Resource, Debug, Default)]
pub struct PreviewWalk {
    pub predict: Predict,
    pub run: RunMods,
    /// The class's charstats speeds; `None`: no charstats row (synthetic
    /// data), so the prediction only follows the model's placements.
    pub speeds: Option<Speeds>,
    /// The walks the link recorded.
    pub tap: WalkTap,
    /// The unit art whose drawn mode follows the prediction, when units
    /// are drawn.
    pub art: Option<SharedUnitArt>,
    /// The server tick count at the last frame.
    seen_ticks: u64,
    /// The prediction and `seen_ticks` before the last [`Self::frame`]
    /// (for [`Self::catch_up`]).
    before: Option<(Predict, u64)>,
    /// The model's local player mode at the last [`Self::frame`] and at
    /// the one before it.
    model_mode: [Option<u32>; 2],
}

impl PreviewWalk {
    pub fn new(tap: WalkTap, speeds: Option<Speeds>) -> Self {
        PreviewWalk {
            tap,
            speeds,
            ..Default::default()
        }
    }

    /// One bridge frame: the recorded walks, then a step if the server
    /// ticked since the last frame.
    pub fn frame(&mut self, world: &ClientWorld) {
        // With the single-player server copy
        // the drawn position is already the server's after the last
        // update, which is where the base game's pump reads a click: no earlier
        // prediction is kept (`render.scene.a3-outdoor-spider-forest`: the
        // click of frame 193 reads update 192's position, not 191's).
        self.before = if world.local().is_some_and(|p| p.precise.is_some()) {
            None
        } else {
            Some((self.predict.clone(), self.seen_ticks))
        };
        self.model_mode = [world.local().map(|p| p.mode), self.model_mode[0]];
        let ticked = world.server_ticks != self.seen_ticks;
        self.seen_ticks = world.server_ticks;
        let walks = self.tap.take();
        let walks_sent = walks.len();
        if self.tap.take_waypoint() {
            self.predict.waypoint_sent();
        }
        match self.speeds {
            Some(speeds) => self.predict.frame(world, walks, ticked, speeds),
            // No speeds: snaps only (no step).
            None => self.predict.observe(world),
        }
        // With the server copy the model's mode is the server's after the
        // update: a neutral mode there while no new walk was sent means the
        // server ended the walk (a click on the sub-tile next to the player
        // stops it at once), so the drawn mode follows it instead of
        // walking on to the target (`render.scene.a5-outdoor-frigid-highlands`:
        // the click of frame 140 stands in town neutral from update 141).
        if ticked && walks_sent == 0 && self.predict.walking().is_some() {
            if let Some(p) = world.local().filter(|p| p.precise.is_some()) {
                use crate::bridge::modes::player_mode::{NEUTRAL, TOWN_NEUTRAL};
                if matches!(p.mode, NEUTRAL | TOWN_NEUTRAL) {
                    self.predict.stop();
                }
            }
        }
    }

    /// The base game's message pump runs a click before the client update of its
    /// frame, so the walk it sends steps in that update. A D2RUST pass steps
    /// the prediction first (`PreUpdate`) and handles the click after, so
    /// when this pass's clicks sent a walk the step is redone with it
    /// (measured: the click of frame 42 steps at update 42).
    /// Returns whether the prediction was redone.
    pub fn catch_up(&mut self, world: &ClientWorld) -> bool {
        if !self.tap.pending() {
            return false;
        }
        // With the single-player server copy
        // the drawn position is the server's after each update, so the
        // redo would only start the walk pose one update ahead of the base game's:
        // the click's walk steps at the next update instead (PR #81).
        if world.local().is_some_and(|p| p.precise.is_some()) {
            return false;
        }
        let Some((predict, seen)) = self.before.take() else {
            return false;
        };
        self.predict = predict;
        self.seen_ticks = seen;
        self.frame(world);
        true
    }

    /// The position a click reads: the prediction before this pass's step
    /// (the base game's pump runs before the update); with the server copy, the
    /// current position (see [`Self::frame`]).
    pub fn pump_position(&self) -> Option<(u32, u32)> {
        match &self.before {
            Some((p, _)) => p.position(),
            None => self.predict.position(),
        }
    }

    /// The class walk / run speeds (`None`: no class row).
    pub fn speeds(&self) -> Option<Speeds> {
        self.speeds
    }

    /// The predicted position keyed by the local player, for the feed.
    pub fn local_at(&self) -> Option<(crate::bridge::UnitKey, (u32, u32))> {
        Some((self.predict.player()?, self.predict.position()?))
    }

    /// The local player's mode for the unit sounds: the prediction's walk /
    /// run mode while it walks. A click's walk the prediction has not
    /// stepped yet (it steps at the next update, [`Self::catch_up`]) while
    /// the server copy already shows the model's walk mode: the neutral
    /// mode, as the base game's client unit enters the walk only at that next
    /// update (measured: the first footstep 2720 at update 21, not 20).
    /// Settled by a recorded town walk's audio, which matches.
    /// `None`: the model's mode.
    pub fn sound_mode(&self, world: &ClientWorld) -> Option<(crate::bridge::UnitKey, u32)> {
        let key = self.predict.player()?;
        if let Some(m) = self.predict.mode() {
            return Some((key, m));
        }
        let p = world.local().filter(|p| p.key == key)?;
        if !self.tap.pending() || !matches!(p.mode, 2 | 3 | 6) {
            return None;
        }
        // A click while the player already walks: the base game's client unit stays
        // in its walk and its animation runs on (no restart; `act4out`
        // re-click at frame 76: player footsteps at 73, 78, 83).
        if matches!(self.model_mode[1], Some(2 | 3 | 6)) {
            return None;
        }
        Some((key, crate::bridge::modes::neutral_walk(world, key).0))
    }
}

/// Runs [`PreviewWalk::frame`] after the bridge frame and hands the
/// prediction to the feed and the unit art.
pub fn preview_walk_frame(
    mut bridge: ResMut<BridgeResource>,
    mut walk: ResMut<PreviewWalk>,
    mut state: ResMut<WorldViewState>,
) {
    // The living monsters' footprints for the
    // client path (at their model positions).
    let others = other_units(bridge.0.world(), &bridge.0.inputs().tables.monsters);
    let objects = other_objects(bridge.0.world(), &bridge.0.inputs().objclient.rows);
    walk.predict.set_others(others, objects);
    let was_walking = walk.predict.mode().is_some();
    // The update's aim of the local player: a
    // walk to a unit aims along the client plan made before this frame
    // (fx-charged-bolt-cast ticks 123-126); the click tick and a point
    // walk keep the server copy's.
    bridge.0.aim_local(walk.predict.unit_new_direction());
    walk.frame(bridge.0.world());
    // The walk ended: the neutral animation starts over (the model's
    // free-running neutral frame was not the one the base game plays).
    if was_walking && walk.predict.mode().is_none() {
        bridge.0.restart_local_neutral();
    }
    // Where the local player stands, for the units that face it (B025).
    bridge.0.set_local_predicted(walk.predict.position());
    publish_prediction(bridge.0.world(), &walk, state.feed.as_mut());
    // The push-out's placement clears the weather pools
    // (placement clears).
    let world = bridge.0.world();
    let pushed = local_push_out(
        world,
        &bridge.0.inputs().objclient.rows,
        walk.local_at(),
        walk.sound_mode(world),
    );
    state.feed.set_local_push(pushed.is_some());
}

/// Hands the prediction to the feed and the unit art.
pub fn publish_prediction(
    world: &ClientWorld,
    walk: &PreviewWalk,
    feed: &mut dyn super::feed::ViewFeed,
) {
    feed.set_local_prediction(walk.local_at());
    if let Some(art) = &walk.art {
        let mut art = art.write().unwrap_or_else(|e| e.into_inner());
        art.pose_mode = walk.predict.player().and_then(|k| {
            let mode = walk
                .predict
                .mode()
                .or_else(|| walk.predict.stood_mode(world))?;
            Some((k, mode))
        });
        art.pose_dir = walk
            .predict
            .player()
            .and_then(|k| Some((k, walk.predict.facing()?)));
        let speed = match walk.predict.mode() {
            Some(3) => super::unit_rules::player_run_speed(walk.speeds),
            _ => super::unit_rules::PLAYER_WALK_SPEED,
        };
        let since = walk
            .predict
            .player()
            .and_then(|k| Some((k, walk.predict.walk_since()?, speed)));
        // The prediction arrived but the drawn mode is still the walk
        // (the model leaves it when the server's stop comes): the base game's
        // walk frame keeps running from the walk's start (fxnova tick 55:
        // `cur` 885 = 33 · 213 mod 2048, not the tick phase).
        let drawn = |k| match art.pose_mode {
            Some((p, m)) if p == k => Some(m),
            _ => world.units.get(&k).map(|u| u.mode),
        };
        let kept = art
            .pose_since
            .filter(|&(k, ..)| drawn(k).is_some_and(|m| matches!(m, 2 | 3 | 6)));
        art.pose_since = since.or(kept);
    }
}

/// The model's monsters not dying or dead (mode 0, 12) with a cell and a
/// `monstats2` row: their footprint inputs.
pub fn other_units(
    world: &ClientWorld,
    classes: &[Option<crate::bridge::world::MonsterClass>],
) -> Vec<crate::bridge::client_path::OtherUnit> {
    world
        .units
        .values()
        .filter(|u| u.key.unit_type == crate::bridge::world::MONSTER && !matches!(u.mode, 0 | 12))
        .filter_map(|u| {
            let (x, y) = u.position?;
            let c = classes.get(u.class as usize)?.as_ref()?;
            Some(crate::bridge::client_path::OtherUnit {
                x,
                y,
                size_x: c.size_x,
                npc: c.npc,
                in_town: c.in_town,
                interact: c.interact,
            })
        })
        .collect()
}

/// The model's objects whose footprint is on the client grid (stamped by
/// the 0x51 object init when `HasCollision[mode]` ≠ 0, not
/// yet freed: [`crate::bridge::world::ObjectData::footprint`]) with an
/// `objects` row: their footprint inputs
/// (`bridge::client_path`).
pub fn other_objects(
    world: &ClientWorld,
    rows: &[crate::bridge::objects::ObjClientRow],
) -> Vec<crate::bridge::client_path::OtherObject> {
    world
        .units
        .values()
        .filter(|u| {
            u.key.unit_type == crate::bridge::world::OBJECT
                && matches!(&u.kind, crate::bridge::world::KindData::Object(d) if d.footprint)
        })
        .filter_map(|u| {
            let (x, y) = u.position?;
            let r = rows.get(u.class as usize)?;
            Some(crate::bridge::client_path::OtherObject {
                x,
                y,
                shape: r.shape,
            })
        })
        .collect()
}

/// The local player's point after the base-game client's push-out
/// ([`crate::bridge::client_path::push_out`]), as 16.16 subtiles (the
/// teleport puts it at the sub-tile centre): the player at
/// `local_at` (the preview's point) or its model cell, in `mode` (the
/// drawn mode) or its model mode; `None` outside modes 1 and 5, without a
/// client DRLG, or with no collision.
pub fn local_push_out(
    world: &ClientWorld,
    rows: &[crate::bridge::objects::ObjClientRow],
    local_at: Option<(crate::bridge::UnitKey, (u32, u32))>,
    mode: Option<(crate::bridge::UnitKey, u32)>,
) -> Option<(crate::bridge::UnitKey, (u32, u32))> {
    let key = world.local_player?;
    let u = world.units.get(&key)?;
    let m = mode.filter(|&(k, _)| k == key).map_or(u.mode, |(_, m)| m);
    if !matches!(m, 1 | 5) {
        return None;
    }
    let at = match local_at.filter(|&(k, _)| k == key) {
        Some((_, (x16, y16))) => ((x16 >> 16) as u16, (y16 >> 16) as u16),
        None => u.position?,
    };
    let drlg = &world.drlg.as_ref()?.drlg;
    let t = crate::bridge::predict::path_tables()?;
    let objects = other_objects(world, rows);
    let (x, y) = crate::bridge::client_path::push_out(t, drlg, &objects, at)?;
    let centre = |v: i32| ((v as u32) << 16) | d2_sim::path::coords::FP16_CENTER;
    Some((key, (centre(x), centre(y))))
}

/// The systems of the wiring below, once per app (`app::session`).
pub fn register_preview_walk(app: &mut App) {
    super::present::configure_preview_order(app);
    app.add_systems(
        PreUpdate,
        preview_walk_frame
            .in_set(super::present::PreviewOrder::PlayerWalk)
            .run_if(resource_exists::<BridgeResource>)
            .run_if(resource_exists::<WorldViewState>)
            .run_if(resource_exists::<PreviewWalk>),
    );
}

/// Adds `walk` and [`preview_walk_frame`] (after the bridge frame, before
/// the unit mirror and the unit art loader).
pub fn add_preview_walk(app: &mut impl crate::app::session::Setup, walk: PreviewWalk) {
    app.register("preview-walk", register_preview_walk);
    app.world_mut().insert_resource(walk);
}

#[cfg(test)]
mod catch_up_tests {
    use super::*;

    #[test]
    fn no_pending_walk_leaves_the_step_alone() {
        let mut w = PreviewWalk::default();
        let world = ClientWorld::default();
        w.frame(&world);
        assert!(!w.catch_up(&world));
        assert_eq!(w.pump_position(), None);
    }

    /// A click's walk the prediction has not stepped yet: the model's walk
    /// mode (the server copy) is not the sounds' mode yet, the neutral is;
    /// with no walk pending the model's mode stands.
    #[test]
    fn an_unstepped_click_walk_sounds_neutral() {
        use crate::bridge::world::ClientUnit;
        use crate::bridge::UnitKey;
        let key = UnitKey::new(0, 1);
        let mut world = ClientWorld::default();
        let mut u = ClientUnit::new(key);
        u.position = Some((100, 100));
        u.server_point = (100, 100);
        world.units.insert(key, u);
        world.local_player = Some(key);
        let mut w = PreviewWalk::default();
        w.frame(&world);
        assert_eq!(w.predict.player(), Some(key));
        assert_eq!(w.sound_mode(&world), None, "standing: the model's mode");
        // The click sends a walk; the server copy already walks (mode 2).
        w.tap.record(&[1, 110, 0, 100, 0]);
        world.units.get_mut(&key).unwrap().mode = 2;
        assert_eq!(w.sound_mode(&world), Some((key, 1)), "not stepped: neutral");
        // Without a pending walk the model's walk mode is the sounds'.
        w.tap.take();
        assert_eq!(w.sound_mode(&world), None);
    }
}
