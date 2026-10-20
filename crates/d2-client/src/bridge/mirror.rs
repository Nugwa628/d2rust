//! Bevy mirror: the bridge as a resource, bounded tick catch-up per Bevy
//! frame in `PreUpdate`, and entities that mirror the client world
//! model's units. Components here are views, never game state.

use std::collections::BTreeMap;

use bevy::prelude::*;

use super::link::ServerLink;
use super::output::Output;
use super::world::{ClientUnit, UnitKey};
use super::Bridge;

/// The link type the Bevy app holds.
pub type DynLink = Box<dyn ServerLink + Send + Sync>;

/// The bridge, owned by the Bevy world. Insert it before the first frame;
/// the plugin's systems run only while it exists.
#[derive(Resource)]
pub struct BridgeResource(pub Bridge<DynLink>);

/// View of one client unit: overwritten from the model.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct UnitView {
    pub key: UnitKey,
}

impl UnitView {
    fn of(unit: &ClientUnit) -> Self {
        Self { key: unit.key }
    }
}

/// The outputs of the last bridge frame, handed over whole
/// at the end of [`bridge_frame`]; the output dispatcher takes them
/// before the input and UI systems. A frame replaces what an earlier
/// frame left, so one frame's outputs never mix with the next frame's.
#[derive(Resource, Default, Debug)]
pub struct FrameOutputs(pub Vec<Output>);

/// Mirror entity of each unit key.
#[derive(Resource, Default, Debug)]
pub struct MirrorIndex(pub BTreeMap<UnitKey, Entity>);

/// Adds the bridge frame and the mirror to `PreUpdate`, in that order.
pub struct BridgePlugin;

impl Plugin for BridgePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MirrorIndex>()
            .init_resource::<FrameOutputs>()
            .add_systems(
                PreUpdate,
                (bridge_frame, mirror_units)
                    .chain()
                    .run_if(resource_exists::<BridgeResource>),
            );
    }
}

/// A scripted host clock ("headless
/// tests and replays pass a scripted value"): when present,
/// [`bridge_frame`] gives the bridge the clock's value (wrapping
/// milliseconds) instead of the real time. A headless rig shares its step
/// clock (the server's) here, so nothing it does depends on wall time.
#[derive(Resource, Debug, Clone, Default)]
pub struct ScriptedClock(pub std::sync::Arc<std::sync::atomic::AtomicU32>);

impl ScriptedClock {
    /// The clock's value now.
    pub fn now(&self) -> u32 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The last server tick the world view drew, when the app paces the
/// bridge to its draws (`play --dump-draws`):
/// while the bridge's tick is newer, [`bridge_frame`] runs a
/// held frame ([`Bridge::held_frame`]), so every server tick is drawn
/// once and none is skipped while the GPU still holds the previous frame.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct DrawnTick(pub u64);

/// A check run's input-only pass: set
/// by the script after a drawn tick N whose next tick is not drawn while
/// steps of `frame N + 1` are due; the next pass then holds the server
/// frame ([`Bridge::held_frame`]) and only runs those steps, so they reach
/// the server before tick N + 1, as the base game's message pump takes them before
/// that tick's update.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct InputOnlyPass(pub bool);

/// Whether the next [`bridge_frame`] is a held one: an input-only pass, or
/// the app's draws behind the bridge's tick.
pub fn held<L: ServerLink>(
    bridge: &Bridge<L>,
    drawn: Option<&DrawnTick>,
    input_only: Option<&InputOnlyPass>,
) -> bool {
    input_only.is_some_and(|i| i.0) || drawn.is_some_and(|d| bridge.world().server_ticks > d.0)
}

/// One bridge frame, then its outputs are handed over (step 4). Errors
/// go to Bevy's error handler.
pub fn bridge_frame(
    mut bridge: ResMut<BridgeResource>,
    mut outputs: ResMut<FrameOutputs>,
    scripted: Option<Res<ScriptedClock>>,
    time: Option<Res<Time<Real>>>,
    drawn: Option<Res<DrawnTick>>,
    input_only: Option<Res<InputOnlyPass>>,
) -> Result {
    if held(&bridge.0, drawn.as_deref(), input_only.as_deref()) {
        bridge.0.held_frame();
        outputs.0 = bridge.0.take_outputs();
        return Ok(());
    }
    // The host clock as `GetTickCount`:
    // a scripted value when the app sets one ([`ScriptedClock`]), else
    // wrapping milliseconds since the app started. An app with neither
    // keeps the value it set.
    if let Some(clock) = scripted {
        bridge.0.set_now(clock.now());
    } else if let Some(time) = time {
        bridge.0.set_now(time.elapsed().as_millis() as u32);
    }
    bridge.0.display_frame(drawn.is_none())?;
    outputs.0 = bridge.0.take_outputs();
    Ok(())
}

/// Spawns, updates and despawns mirror entities from the model (step
/// 3), in key order.
pub fn mirror_units(
    mut commands: Commands,
    bridge: Res<BridgeResource>,
    mut index: ResMut<MirrorIndex>,
    mut views: Query<&mut UnitView>,
) {
    let units = &bridge.0.world().units;
    index.0.retain(|key, &mut entity| {
        let keep = units.contains_key(key);
        if !keep {
            commands.entity(entity).despawn();
        }
        keep
    });
    for (key, unit) in units {
        let view = UnitView::of(unit);
        match index.0.get(key) {
            Some(&entity) => {
                if let Ok(mut v) = views.get_mut(entity) {
                    v.set_if_neq(view);
                }
            }
            None => {
                index.0.insert(*key, commands.spawn(view).id());
            }
        }
    }
}
