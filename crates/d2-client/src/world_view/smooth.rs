//! Presentation timing and layers between simulation ticks.
//!
//! World camera sliding is disabled until the world can be rendered with
//! overscan. Sliding a screen-sized layer exposes stale pixels at the
//! trailing edges and shifts the bottom-bar boundary. The camera and local
//! player now use the current tick's positions at every display frame.
//! At the 60 Hz game rate this gives 60 camera updates per second.
//!
//! Other units retain their between-tick interpolation and layer
//! recomposition. The timing and camera APIs remain for future overscan;
//! the previous-world fallback stays inert because no step is blended.
//!
//! With the native UI pass, one quad shows the tick frame or recomposed
//! layers with the UI drawn over it at the output resolution.

use std::time::Duration;

use bevy::camera::visibility::RenderLayers;
use bevy::image::Image;
use bevy::math::Vec2;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};

use crate::rules::camera::Camera as ViewCamera;
use crate::scene::order::pass;
use crate::scene::{DrawItem, ItemTag};

use super::native_ui::{NativeUiMaterial, NativeUiQuad};
use super::node::{ComposeJob, LayerTargets};
use super::present::{WorldViewSprite, WorldViewState, WorldViewTarget, PRESENT_LAYER};
use super::sharp::{SharpMaterial, FULL_IMAGE};
use bevy::sprite_render::AlphaMode2d;

/// One tick of the game logic at the base rate of 25 ticks a second.
pub const TICK: Duration = Duration::from_millis(40);

/// A camera step longer than this on either axis (frame pixels) is a jump,
/// not walking: the view snaps. Running covers well under this per tick.
pub const SNAP_PX: i32 = 64;

/// Ticks the drawn frame may advance at once and still blend (a draw that
/// waited for the GPU skips a tick); more than this snaps.
pub const MAX_TICK_GAP: u64 = 2;

/// Sliding needs real world pixels beyond every frame edge. Keep it off
/// until overscan is available; 60 Hz ticks drive the camera meanwhile.
const WORLD_SLIDE: bool = false;

/// Units blend between ticks once the display shows at least this many
/// frames per tick, and stop below [`BLEND_OFF`] (the gap keeps a display
/// rate near the threshold from switching back and forth). Below it most
/// display frames bring a new tick anyway: a blend would only redraw the
/// frames that bring none, at a position from the tick before, so the
/// units would step back and forth, at the cost of an extra compose.
pub const BLEND_ON: f32 = 1.4;
/// See [`BLEND_ON`].
pub const BLEND_OFF: f32 = 1.15;

/// How much of each new display frame interval goes into the smoothed one
/// ([`SmoothCamera::note_frame`]).
const FRAME_DT_WEIGHT: f32 = 0.1;

/// The first display frame interval counts as at most this long.
const FRAME_DT_MAX: f32 = 0.1;

/// Whether a frame item stays fixed on screen while the camera blends:
/// everything drawn after the world (the screen fade and the UI pass:
/// panels, the automap, labels), and the local player (its body and
/// shadow carry its GUID). The camera keeps the local player in place on
/// screen, so moving it with the ground would make it wobble between
/// ticks. A ground item whose GUID equals the player's would stay put for
/// the length of one tick, which is harmless.
pub fn on_screen_layer(item: &DrawItem, local_player: Option<u32>) -> bool {
    if item.key.pass() >= pass::SCREEN_FADE {
        return true;
    }
    match item.tag {
        ItemTag::Ui(_) => true,
        ItemTag::Unit(guid) => Some(guid) == local_player,
        ItemTag::None | ItemTag::Tile { .. } => false,
    }
}

/// The camera history and the time base of the blend.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct SmoothCamera {
    /// The server tick of the last drawn frame and its camera.
    last: Option<(u64, Option<ViewCamera>)>,
    /// When that frame was first presented (app time).
    at: Duration,
    /// How long the blend takes: the game's tick period times the ticks
    /// the last drawn tick moved on (at most [`MAX_TICK_GAP`]). The ticks
    /// run on a steady clock; when a drawn tick was first presented
    /// varies by up to a display frame, so measuring the time between
    /// them would make the units speed up and slow down from tick to tick.
    span: Duration,
    rate: d2_sim::tick::TickRate,
    /// The last camera step, frame pixels (the unit origin's move), when
    /// it is blended.
    step: Option<IVec2>,
    /// The start of the last display frame ([`Self::note_frame`]).
    frame_at: Option<Duration>,
    /// The display frame interval, smoothed, in seconds.
    frame_dt: Option<f32>,
    /// Whether units blend between ticks ([`BLEND_ON`]).
    blend_units: bool,
}

impl SmoothCamera {
    /// Notes the drawn frame of `tick` with its `camera`, seen at `now`.
    /// A tick seen again changes nothing.
    pub fn observe(&mut self, tick: u64, camera: Option<ViewCamera>, now: Duration) {
        if self.last.is_some_and(|(t, _)| t == tick) {
            return;
        }
        let gap = self.last.map_or(0, |(t, _)| tick.wrapping_sub(t));
        self.step = match (self.last.and_then(|(_, c)| c), camera) {
            (Some(from), Some(to)) if WORLD_SLIDE && (1..=MAX_TICK_GAP).contains(&gap) => {
                step(&from, &to)
            }
            _ => None,
        };
        let tick_time = Duration::from_secs_f64(1.0 / f64::from(self.rate.hz()));
        self.span = tick_time * gap.clamp(1, MAX_TICK_GAP) as u32;
        self.last = Some((tick, camera));
        self.at = now;
    }

    /// Notes a display frame starting at `now`: the smoothed frame
    /// interval, and from it whether units blend between ticks.
    pub fn note_frame(&mut self, now: Duration) {
        // One long frame counts as at most twice the usual one, so a hitch
        // does not switch the blend off.
        let longest = self.frame_dt.map_or(FRAME_DT_MAX, |dt| dt * 2.0);
        let dt = self
            .frame_at
            .map(|at| now.saturating_sub(at).as_secs_f32().min(longest));
        self.frame_at = Some(now);
        // A frame at the same time (a fixed display clock's first frame)
        // says nothing about the rate.
        let Some(dt) = dt.filter(|&dt| dt > 0.0) else {
            return;
        };
        let smoothed = match self.frame_dt {
            Some(old) => old + (dt - old) * FRAME_DT_WEIGHT,
            None => dt,
        };
        self.frame_dt = Some(smoothed);
        let per_tick = 1.0 / (self.rate.hz() as f32 * smoothed);
        if per_tick >= BLEND_ON {
            self.blend_units = true;
        } else if per_tick < BLEND_OFF {
            self.blend_units = false;
        }
    }

    /// Whether units blend between ticks: the display shows clearly more
    /// frames than there are ticks ([`BLEND_ON`]).
    pub fn units_blend(&self) -> bool {
        self.blend_units
    }

    /// How far the display has come from the previous tick's camera to
    /// the current one at `now`: 0 right after the tick frame was drawn,
    /// 1 once the blend span has passed.
    pub fn alpha(&self, now: Duration) -> f32 {
        let span = self.span.max(Duration::from_millis(1));
        (now.saturating_sub(self.at).as_secs_f32() / span.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// The world layer's offset at `now`, frame pixels (x right, y down):
    /// the part of the last camera step not yet shown. The content of the
    /// tick frame sits where the camera's move put it; showing the view
    /// from a camera `(1 − alpha)` of a step behind moves it back by that
    /// much. Zero when no step is blended.
    pub fn offset(&self, now: Duration) -> Vec2 {
        match self.step {
            Some(d) => d.as_vec2() * (1.0 - self.alpha(now)),
            None => Vec2::ZERO,
        }
    }

    /// The offset at `now` of the tick before's world layer
    /// ([`SmoothLayers::prev`]), placed so it lines up with the shifted
    /// world layer: that layer was drawn from a camera one step behind, so
    /// its picture sits one step further along. `None` when no step is
    /// blended.
    pub fn prev_offset(&self, now: Duration) -> Option<Vec2> {
        self.step.map(|d| self.offset(now) - d.as_vec2())
    }
}

/// The `uv_rect` of the tick before's world layer at `now`, or
/// [`NO_PREV`] when no step is blended. The shaders read it only where the
/// shifted world layer has no picture (its uv is outside the image): the
/// strip the slide uncovers at the trailing edges, which the tick before's
/// layer still covers.
pub fn prev_uv(smooth: &SmoothCamera, size: Vec2, now: Duration) -> Vec4 {
    smooth
        .prev_offset(now)
        .map_or(NO_PREV, |o| shifted_uv(size, o))
}

/// No fill from the tick before (a zero-sized rectangle): the shaders keep
/// the world layer's clamped edge.
pub const NO_PREV: Vec4 = Vec4::ZERO;

/// The camera step from `from` to `to` in frame pixels, or `None` when it
/// must snap (a jump or a changed view).
fn step(from: &ViewCamera, to: &ViewCamera) -> Option<IVec2> {
    if from.size != to.size || from.view != to.view {
        return None;
    }
    let d = IVec2::new(to.unit.x - from.unit.x, to.unit.y - from.unit.y);
    (d.x.abs() <= SNAP_PX && d.y.abs() <= SNAP_PX).then_some(d)
}

/// The sharp material's `uv_rect` that shows a layer of `size` frame
/// pixels moved by `offset` (frame pixels): the image pixel at
/// `p − offset` lands on frame pixel `p`. The strip this uncovers at the
/// trailing edges (at most one step wide) is filled from the tick
/// before's world layer ([`prev_uv`]).
pub fn shifted_uv(size: Vec2, offset: Vec2) -> Vec4 {
    let origin = -offset / size;
    Vec4::new(origin.x, origin.y, 1.0, 1.0)
}

/// The presentation layers' images.
#[derive(Resource, Clone, Debug)]
pub struct SmoothLayers {
    pub world: Handle<Image>,
    pub screen: Handle<Image>,
    /// The tick before's world layer ([`LayerTargets::prev`]).
    pub prev: Handle<Image>,
}

impl SmoothLayers {
    pub fn targets(&self) -> LayerTargets {
        LayerTargets {
            world: self.world.clone(),
            screen: self.screen.clone(),
            prev: self.prev.clone(),
        }
    }
}

/// A presentation layer's quad: `true` for the screen layer.
#[derive(Component, Debug, Clone, Copy)]
pub struct SmoothLayer {
    pub screen: bool,
}

/// A GPU target image of the frame's size, as the presented frame's,
/// sampled as the sharp material samples the frame.
fn layer_image(size: UVec2) -> Image {
    let mut image = Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    image.sampler = super::sharp::frame_sampler();
    image
}

/// Notes each display frame (the display rate) and each new drawn tick
/// and its camera, at the time it is first presented: the time base of the
/// camera blend and of the units' (`super::unit_motion`).
pub(crate) fn observe_camera(
    mut smooth: ResMut<SmoothCamera>,
    state: Res<WorldViewState>,
    time: Res<Time<Real>>,
    bridge: Option<Res<crate::bridge::BridgeResource>>,
) {
    smooth.rate = bridge
        .as_ref()
        .map_or(d2_sim::tick::TickRate::BASE, |b| b.0.world().tick_rate);
    smooth.note_frame(time.elapsed());
    if let Some(last) = &state.last {
        let camera = *state.camera.read().unwrap_or_else(|e| e.into_inner());
        smooth.observe(last.server_tick, camera, time.elapsed());
    }
}

/// Presents the layers on the display frames between ticks: shows the
/// world layer shifted and the screen layer on top while the camera or a
/// unit is between two ticks, drawn through the sharp scaler and placed
/// like the presented frame (a sub-pixel slide stays crisp at 1.8x, 2.4x
/// and 3.6x). Creates the layers once the GPU frame is a sharp quad; the
/// next job composes into them.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn present_layers(
    mut commands: Commands,
    smooth: Res<SmoothCamera>,
    moved: Option<Res<super::unit_motion::MotionShown>>,
    target: Option<Res<WorldViewTarget>>,
    layers: Option<Res<SmoothLayers>>,
    job: Option<Res<ComposeJob>>,
    time: Res<Time<Real>>,
    mut images: ResMut<Assets<Image>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<SharpMaterial>>>,
    main: Query<
        (&Transform, &MeshMaterial2d<SharpMaterial>),
        (With<WorldViewSprite>, Without<SmoothLayer>),
    >,
    mut quads: Query<(
        &SmoothLayer,
        &MeshMaterial2d<SharpMaterial>,
        &mut Transform,
        &mut Visibility,
    )>,
) {
    let now = time.elapsed();
    let (Some(target), Some(mut meshes), Some(mut materials)) = (target, meshes, materials) else {
        return;
    };
    let Ok((main_at, main_material)) = main.get(target.sprite) else {
        return;
    };
    let Some(scale) = materials.get(&main_material.0).map(|m| m.scale) else {
        return;
    };
    let Some(size) = images.get(&target.image).map(|i| i.size()) else {
        return;
    };
    if layers.is_none() {
        // The GPU path only: its jobs carry the layer targets.
        if job.is_some() {
            let world = images.add(layer_image(size));
            let screen = images.add(layer_image(size));
            let prev = images.add(layer_image(size));
            let quad = meshes.add(Rectangle::new(size.x as f32, size.y as f32));
            for (image, screen_layer, z) in [(&world, false, 1.0), (&screen, true, 2.0)] {
                let mut material = SharpMaterial::new(image.clone());
                material.scale = scale;
                if screen_layer {
                    material.alpha_mode = AlphaMode2d::Blend;
                } else {
                    material.prev = Some(prev.clone());
                }
                commands.spawn((
                    Mesh2d(quad.clone()),
                    MeshMaterial2d(materials.add(material)),
                    Transform::from_xyz(0.0, 0.0, z),
                    Visibility::Hidden,
                    SmoothLayer {
                        screen: screen_layer,
                    },
                    RenderLayers::layer(PRESENT_LAYER),
                    crate::app::session::GameEntity,
                ));
            }
            commands.insert_resource(SmoothLayers {
                world,
                screen,
                prev,
            });
        }
        return;
    }
    // The layers hold the presented tick only when its job composed them.
    let composed = job.as_ref().is_some_and(|j| j.layers.is_some());
    let offset = smooth.offset(now);
    // Shown while the camera or a unit is between two ticks; otherwise
    // the tick frame itself is presented. With the native UI pass its
    // quad shows them instead ([`present_native`]).
    let units = moved.is_some_and(|m| m.0);
    let native = job.as_ref().is_some_and(|j| j.ui.is_some());
    let show = composed && !native && (offset != Vec2::ZERO || units);
    for (layer, material, mut at, mut visibility) in &mut quads {
        let wanted = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        visibility.set_if_neq(wanted);
        if !show {
            continue;
        }
        let z = at.translation.z;
        *at = *main_at;
        at.translation.z = z;
        let (uv_rect, prev_rect) = if layer.screen {
            (FULL_IMAGE, NO_PREV)
        } else {
            (
                shifted_uv(size.as_vec2(), offset),
                prev_uv(&smooth, size.as_vec2(), now),
            )
        };
        // Touching a material re-uploads it: only on a change.
        if materials
            .get(&material.0)
            .is_some_and(|m| m.uv_rect != uv_rect || m.prev_uv != prev_rect || m.scale != scale)
        {
            if let Some(mut m) = materials.get_mut(&material.0) {
                m.uv_rect = uv_rect;
                m.prev_uv = prev_rect;
                m.scale = scale;
            }
        }
    }
}

/// Presents the frame with the native UI pass's UI over it, when the
/// presented job has the pass (`super::native_ui`): one quad on the
/// frame's place, drawn over the frame and the smooth camera's layers. It
/// shows the tick frame (the scene, without its UI), or while the camera
/// or a unit is between two ticks the world layer shifted under the screen
/// layer, as [`present_layers`] would; and the UI on top. Hidden when the
/// job has no pass: the frame then carries its own UI.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn present_native(
    mut commands: Commands,
    smooth: Res<SmoothCamera>,
    moved: Option<Res<super::unit_motion::MotionShown>>,
    target: Option<Res<WorldViewTarget>>,
    layers: Option<Res<SmoothLayers>>,
    job: Option<Res<ComposeJob>>,
    time: Res<Time<Real>>,
    images: Res<Assets<Image>>,
    (meshes, sharp, natives): (
        Option<ResMut<Assets<Mesh>>>,
        Option<Res<Assets<SharpMaterial>>>,
        Option<ResMut<Assets<NativeUiMaterial>>>,
    ),
    main: Query<
        (&Transform, &MeshMaterial2d<SharpMaterial>),
        (
            With<WorldViewSprite>,
            Without<SmoothLayer>,
            Without<NativeUiQuad>,
        ),
    >,
    mut quads: Query<
        (
            &MeshMaterial2d<NativeUiMaterial>,
            &mut Transform,
            &mut Visibility,
        ),
        With<NativeUiQuad>,
    >,
) {
    let ui = job.as_ref().and_then(|j| j.ui.clone());
    let Some(ui) = ui else {
        for (_, _, mut visibility) in &mut quads {
            visibility.set_if_neq(Visibility::Hidden);
        }
        return;
    };
    let (Some(target), Some(mut meshes), Some(sharp), Some(mut natives)) =
        (target, meshes, sharp, natives)
    else {
        return;
    };
    let Ok((main_at, main_material)) = main.get(target.sprite) else {
        return;
    };
    let Some((frame, scale)) = sharp
        .get(&main_material.0)
        .map(|m| (m.frame.clone(), m.scale))
    else {
        return;
    };
    let Some(size) = images.get(&target.image).map(|i| i.size()) else {
        return;
    };
    let composed = job.as_ref().is_some_and(|j| j.layers.is_some());
    let offset = smooth.offset(time.elapsed());
    let units = moved.is_some_and(|m| m.0);
    let show_layers = composed && (offset != Vec2::ZERO || units);
    let (world, screen, prev) = match &layers {
        Some(l) => (l.world.clone(), l.screen.clone(), l.prev.clone()),
        None => (frame.clone(), frame.clone(), frame.clone()),
    };
    let want = NativeUiMaterial {
        frame,
        world,
        screen,
        cover: ui.cover.clone(),
        add: ui.add.clone(),
        scale,
        prev,
        world_uv: if show_layers {
            shifted_uv(size.as_vec2(), offset)
        } else {
            FULL_IMAGE
        },
        prev_uv: if show_layers {
            prev_uv(&smooth, size.as_vec2(), time.elapsed())
        } else {
            NO_PREV
        },
        flags: Vec4::new(if show_layers { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0),
    };
    let mut at = *main_at;
    at.translation.z = 3.0;
    if quads.is_empty() {
        let quad = meshes.add(Rectangle::new(size.x as f32, size.y as f32));
        commands.spawn((
            Mesh2d(quad),
            MeshMaterial2d(natives.add(want)),
            at,
            Visibility::Inherited,
            NativeUiQuad,
            RenderLayers::layer(PRESENT_LAYER),
            crate::app::session::GameEntity,
        ));
        return;
    }
    for (material, mut transform, mut visibility) in &mut quads {
        visibility.set_if_neq(Visibility::Inherited);
        if *transform != at {
            *transform = at;
        }
        // Touching a material re-uploads it: only on a change.
        let same = natives.get(&material.0).is_some_and(|m| {
            m.frame == want.frame
                && m.world == want.world
                && m.screen == want.screen
                && m.cover == want.cover
                && m.add == want.add
                && m.scale == want.scale
                && m.world_uv == want.world_uv
                && m.prev == want.prev
                && m.prev_uv == want.prev_uv
                && m.flags == want.flags
        });
        if !same {
            if let Some(mut m) = natives.get_mut(&material.0) {
                *m = want.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::camera::{ClientPos, FrameSize, OpenMode};
    use crate::scene::{DrawKey, FrameId};

    fn cam(x: i32, y: i32) -> ViewCamera {
        ViewCamera::new(
            FrameSize::play(),
            OpenMode::default(),
            ClientPos { x, y },
            (0, 0),
        )
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[track_caller]
    fn near(a: Vec2, b: Vec2) {
        assert!((a - b).length() < 1e-4, "{a} vs {b}");
    }

    #[test]
    fn world_offset_and_previous_fill_stay_disabled_at_every_time() {
        for hz in [25, 60] {
            let mut s = SmoothCamera {
                rate: d2_sim::tick::TickRate::new(hz).unwrap(),
                ..SmoothCamera::default()
            };
            let mut tick = 1;
            for dx in [-SNAP_PX - 1, -SNAP_PX, 0, SNAP_PX, SNAP_PX + 1] {
                for dy in [-SNAP_PX, 0, SNAP_PX] {
                    s.observe(tick, Some(cam(100, 50)), ms(tick * 40));
                    tick += 1;
                    s.observe(tick, Some(cam(100 + dx, 50 + dy)), ms(tick * 40));
                    for elapsed in [0, 1, 8, 16, 20, 39, 40, 80] {
                        let now = ms(tick * 40 + elapsed);
                        assert_eq!(s.offset(now), Vec2::ZERO);
                        assert_eq!(s.prev_offset(now), None);
                        assert_eq!(prev_uv(&s, Vec2::new(800.0, 600.0), now), NO_PREV);
                        assert_eq!(
                            shifted_uv(Vec2::new(800.0, 600.0), s.offset(now)),
                            FULL_IMAGE
                        );
                    }
                    tick += 1;
                }
            }
        }
    }

    /// The blend takes one tick period per tick the drawn tick moved on,
    /// however late the tick was first presented (25 Hz: 40 ms a tick).
    #[test]
    fn unit_blend_takes_a_tick_period_per_tick() {
        let mut s = SmoothCamera::default();
        s.observe(1, Some(cam(0, 0)), ms(0));
        s.observe(2, Some(cam(8, 0)), ms(48));
        assert_eq!(s.alpha(ms(58)), 0.25);
        s.observe(3, Some(cam(16, 0)), ms(1048));
        assert_eq!(s.alpha(ms(1068)), 0.5);
        assert_eq!(s.alpha(ms(1088)), 1.0);
        // Two ticks on: twice as long.
        s.observe(5, Some(cam(32, 0)), ms(2000));
        assert_eq!(s.alpha(ms(2040)), 0.5);
    }

    #[test]
    fn the_same_tick_seen_again_keeps_the_time_base() {
        let mut s = SmoothCamera::default();
        s.observe(1, Some(cam(0, 0)), ms(0));
        s.observe(2, Some(cam(4, 0)), ms(40));
        s.observe(2, Some(cam(4, 0)), ms(60));
        assert_eq!(s.alpha(ms(60)), 0.5);
    }

    #[test]
    fn the_shifted_uv_moves_the_content_by_the_offset() {
        let size = Vec2::new(800.0, 600.0);
        assert_eq!(shifted_uv(size, Vec2::ZERO), FULL_IMAGE);
        // Content moved right by 3 px and up by 1.5: frame pixel 3 shows
        // image pixel 0; the whole image's size is kept.
        let r = shifted_uv(size, Vec2::new(3.0, -1.5));
        assert_eq!(r, Vec4::new(-3.0 / 800.0, 1.5 / 600.0, 1.0, 1.0));
        // Frame pixel p (uv p / size) samples image pixel p − offset.
        let p = Vec2::new(3.0, 0.0);
        let uv = r.truncate().truncate() + p / size * r.zw();
        near(uv * size, p - Vec2::new(3.0, -1.5));
    }

    #[test]
    fn local_player_stays_at_the_current_camera_position_between_ticks() {
        use super::super::unit_motion::{shifted_items, Seen, UnitMotion};
        use crate::bridge::world::{UnitKey, MONSTER, PLAYER};
        use crate::rules::camera::moving_to_client;
        use std::collections::BTreeMap;

        let local = UnitKey::new(PLAYER, 1);
        let other = UnitKey::new(MONSTER, 2);
        let pos = (10 << 16, 10 << 16);
        let next = (11 << 16, 10 << 16);
        let camera = |p: (u32, u32)| {
            ViewCamera::new(
                FrameSize::play(),
                OpenMode::default(),
                moving_to_client(p.0, p.1),
                (0, 0),
            )
        };
        let mut s = SmoothCamera::default();
        s.observe(1, Some(camera(pos)), ms(0));
        s.observe(2, Some(camera(next)), ms(40));
        let mut motion = UnitMotion::default();
        let seen = |key, pos| Seen {
            key,
            pos,
            placements: 0,
            on_map: true,
        };
        motion.observe(1, [seen(local, pos), seen(other, pos)]);
        motion.observe(2, [seen(local, next), seen(other, next)]);
        let ground = camera(next).unit_draw(moving_to_client(next.0, next.1), (0, 0));
        let mut movers = BTreeMap::new();
        let items: Vec<_> = [
            (pass::SHADOWS, local),
            (pass::WALLS_UNITS, local),
            (pass::WALLS_UNITS, other),
        ]
        .into_iter()
        .map(|(pass_id, key)| {
            let mut item = DrawItem::new(FrameId(0), ground.0, ground.1);
            item.key = DrawKey::new(pass_id, 0, key.guid, 0).unwrap();
            item.tag = ItemTag::Unit(key.guid);
            movers.insert(item.key.slot(), key);
            item
        })
        .collect();
        for elapsed in 0..40 {
            let now = ms(40 + elapsed);
            let out = shifted_items(
                &items,
                &movers,
                |key| motion.shift(key, s.alpha(now)),
                |i| on_screen_layer(&items[i], Some(local.guid)),
            )
            .expect("other unit still interpolates");
            for item in &out[..2] {
                assert_eq!((item.x, item.y), ground);
                assert_eq!(s.offset(now), Vec2::ZERO);
            }
            assert_ne!((out[2].x, out[2].y), ground);
        }
    }

    #[test]
    fn the_ui_and_the_local_player_stay_on_screen() {
        let item = |pass_id: u32, tag: ItemTag| {
            let mut d = DrawItem::new(FrameId(0), 0, 0);
            d.key = DrawKey::new(pass_id, 0, 0, 0).unwrap();
            d.tag = tag;
            d
        };
        let world = pass::WALLS_UNITS;
        assert!(on_screen_layer(&item(world, ItemTag::Ui(7)), Some(1)));
        assert!(on_screen_layer(&item(world, ItemTag::Unit(1)), Some(1)));
        assert!(on_screen_layer(
            &item(pass::SHADOWS, ItemTag::Unit(1)),
            Some(1)
        ));
        assert!(!on_screen_layer(&item(world, ItemTag::Unit(2)), Some(1)));
        assert!(!on_screen_layer(&item(world, ItemTag::Unit(1)), None));
        let tile = ItemTag::Tile { x: 0, y: 0 };
        assert!(!on_screen_layer(&item(pass::FLOORS, tile), Some(1)));
        assert!(!on_screen_layer(
            &item(pass::LEVEL_BACKGROUND, ItemTag::None),
            Some(1)
        ));
        // The automap's lines carry no tag but draw in the UI pass.
        assert!(on_screen_layer(&item(pass::UI, ItemTag::None), Some(1)));
        assert!(on_screen_layer(
            &item(pass::SCREEN_FADE, ItemTag::None),
            None
        ));
    }
}

#[cfg(test)]
mod display_rate_tests {
    use super::super::unit_motion::{frame_motion, Seen, UnitMotion};
    use super::*;
    use crate::bridge::world::{UnitKey, MONSTER};
    use d2_sim::tick::TickRate;

    const HZ: u32 = 60;

    fn camera() -> SmoothCamera {
        SmoothCamera {
            rate: TickRate::new(HZ).unwrap(),
            ..SmoothCamera::default()
        }
    }

    /// A unit at client pixel `x` (on row 2048), as 16.16 sub-tiles.
    fn at_x(key: UnitKey, x: i32) -> Seen {
        Seen {
            key,
            pos: (((x + 4096) as u32) << 11, ((4096 - x) as u32) << 11),
            placements: 0,
            on_map: true,
        }
    }

    /// Display frames at `fps` for four seconds over 60 Hz ticks of the
    /// wall clock, each frame up to 1 ms early or late: per frame, the x a
    /// unit walking 4 px a tick is drawn at, and whether it blended.
    fn walk(fps: f64) -> Vec<(i32, bool)> {
        let key = UnitKey::new(MONSTER, 2);
        let mut s = camera();
        let mut m = UnitMotion::default();
        let mut out = Vec::new();
        for f in 0..(fps * 4.0) as u64 {
            let jitter = ((f * 7919) % 2001) as f64 / 1e6 - 0.001;
            let t = Duration::from_secs_f64(0.5 + f as f64 / fps + jitter);
            let tick = (t.as_secs_f64() * f64::from(HZ)).floor() as u64;
            let x = tick as i32 * 4;
            s.note_frame(t);
            s.observe(tick, None, t);
            m.observe(tick, [at_x(key, x)]);
            let alpha = frame_motion(&s, t);
            out.push((x + alpha.map_or(0, |a| m.shift(key, a).x), alpha.is_some()));
        }
        out
    }

    /// A unit walking at a steady speed only ever moves forward on screen,
    /// at every display rate. (A tick frame used to show the unit at tick
    /// N and the next display frame the blend from tick N − 1: a step
    /// back, at every tick above the tick rate and on every frame without
    /// a new tick below it.)
    #[test]
    fn units_never_step_back_at_any_display_rate() {
        for fps in [
            24.0, 30.0, 45.0, 50.0, 59.94, 60.0, 75.0, 90.0, 120.0, 144.0, 165.0, 240.0, 360.0,
        ] {
            let drawn = walk(fps);
            for (i, w) in drawn.windows(2).enumerate() {
                assert!(
                    w[1].0 >= w[0].0,
                    "{fps} fps, frame {}: x {} then {}",
                    i + 1,
                    w[0].0,
                    w[1].0
                );
            }
        }
    }

    /// Units blend (more positions than ticks) only where the display
    /// clearly outruns the ticks; at or below the tick rate they are drawn
    /// where the ticks put them, with no compose of their own.
    #[test]
    fn units_blend_only_when_the_display_outruns_the_ticks() {
        for (fps, blends) in [
            (30.0, false),
            (59.94, false),
            (60.0, false),
            (65.0, false),
            (90.0, true),
            (120.0, true),
            (144.0, true),
            (240.0, true),
        ] {
            let drawn = walk(fps);
            let settled = &drawn[drawn.len() / 2..];
            assert!(
                settled.iter().all(|&(_, b)| b == blends),
                "{fps} fps: blend {blends}"
            );
            if blends {
                let distinct: std::collections::BTreeSet<i32> =
                    settled.iter().map(|&(x, _)| x).collect();
                let ticks = settled.len() as f64 * f64::from(HZ) / fps;
                assert!(distinct.len() as f64 > ticks * 1.2, "{fps} fps");
            }
        }
    }

    #[test]
    fn the_blend_switch_has_a_gap_and_ignores_one_hitch() {
        let mut s = camera();
        let mut t = Duration::from_secs(1);
        let mut run = |s: &mut SmoothCamera, fps: f64, frames: u32| {
            for _ in 0..frames {
                t += Duration::from_secs_f64(1.0 / fps);
                s.note_frame(t);
            }
        };
        run(&mut s, 144.0, 200);
        assert!(s.units_blend());
        // 75 fps is 1.25 frames per tick: between the two thresholds.
        run(&mut s, 75.0, 200);
        assert!(s.units_blend());
        run(&mut s, 60.0, 200);
        assert!(!s.units_blend());
        run(&mut s, 75.0, 200);
        assert!(!s.units_blend());
        run(&mut s, 144.0, 200);
        assert!(s.units_blend());
        // One 300 ms hitch counts as 100 ms: still blending afterwards.
        run(&mut s, 1.0 / 0.3, 1);
        run(&mut s, 144.0, 1);
        assert!(s.units_blend());
    }
}

#[cfg(test)]
mod rate_tests {
    use super::*;
    #[test]
    fn blend_span_tracks_the_game_rate() {
        for hz in [25, 60] {
            let mut smooth = SmoothCamera {
                rate: d2_sim::tick::TickRate::new(hz).unwrap(),
                ..SmoothCamera::default()
            };
            let tick = Duration::from_secs_f64(1.0 / f64::from(hz));
            smooth.observe(1, None, Duration::ZERO);
            assert_eq!(smooth.span, tick);
            smooth.observe(2, None, tick * 10);
            assert_eq!(smooth.span, tick);
            smooth.observe(4, None, tick * 10);
            assert_eq!(smooth.span, tick * 2);
            smooth.observe(9, None, tick * 11);
            assert_eq!(smooth.span, tick * MAX_TICK_GAP as u32);
        }
    }
}
