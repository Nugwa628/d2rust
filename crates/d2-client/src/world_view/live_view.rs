//! The live adaptive canvas transaction.
//!
//! A window change asks the authority for the room range of the new frame
//! first. The committed frame, renderer targets, UI geometry and meshes move
//! together only after the matching `ViewReady` arrives. While the authority
//! is answering, or a GPU readback is still in flight, the old canvas stays.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::frame_resize::{resize_resources, FrameTargets};
use super::native_ui::NativeUiTargets;
use super::node::{ComposeJob, MotionJob};
use super::present::{WorldViewGpu, WorldViewSprite, WorldViewState, WorldViewTarget, WorldViewUi};
use super::smooth::SmoothLayers;
use crate::bridge::BridgeResource;
use crate::rules::adaptive_integer::{FrameSelection, ENABLED};
use crate::rules::camera::FrameSize;
use crate::rules::view_sync::{ViewDecision, ViewRequest, ViewSync};
use crate::ui::layout::Screen;

/// Adaptive state kept across window changes.
#[derive(Resource)]
pub struct AdaptiveView {
    /// The configured frame a Fit window returns to.
    pub normal: FrameSize,
    pub sync: ViewSync,
}

impl Default for AdaptiveView {
    fn default() -> Self {
        Self {
            normal: FrameSize::normal_play(),
            sync: ViewSync::default(),
        }
    }
}

/// The requested canvas for the window's physical size.
pub fn selection_for_window(window: UVec2) -> FrameSelection {
    FrameSelection::shipping(
        window.x,
        window.y,
        FrameSize::normal_play(),
        crate::ui::scale_mode(),
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn live_view(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut bridge: ResMut<BridgeResource>,
    mut state: ResMut<WorldViewState>,
    mut view: ResMut<AdaptiveView>,
    gpu: Option<Res<WorldViewGpu>>,
    target: Option<Res<WorldViewTarget>>,
    layers: Option<Res<SmoothLayers>>,
    mut native: Option<ResMut<NativeUiTargets>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut sprites: Query<&mut Mesh2d, With<WorldViewSprite>>,
    mut commands: Commands,
    mut ui: Option<NonSendMut<WorldViewUi>>,
) {
    if !ENABLED {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let window_size = UVec2::new(window.physical_width(), window.physical_height());
    let desired = selection_for_window(window_size);
    let current = FrameSize::play();
    let ack = bridge
        .0
        .world()
        .view_ready
        .map(|(seq, width, height)| ViewRequest {
            seq,
            frame: FrameSize {
                width: width as i32,
                height: height as i32,
            },
        });
    let paused = ui
        .as_ref()
        .and_then(|u| u.original.as_ref())
        .is_some_and(|o| o.is_open(9) || o.is_open(11));
    if paused && desired.frame != current {
        return;
    }
    let in_game = bridge.0.world().in_game;
    let synced = ack.is_some_and(|a| a.frame == current);
    if desired.frame == current && !view.sync.is_pending() {
        if !in_game || synced || paused {
            return;
        }
        // The game is running on a canvas the authority has not confirmed
        // (the window changed while loading): ask for the current frame.
        let request = view.sync.request(current);
        if !send_request(&mut bridge, request) {
            view.sync.send_failed();
        }
        return;
    }
    let decision = if !in_game {
        ViewDecision::Commit(desired)
    } else {
        view.sync.poll(current, desired, ack)
    };
    match decision {
        ViewDecision::Keep => {}
        ViewDecision::Request(request) => {
            if !send_request(&mut bridge, request) {
                view.sync.send_failed();
            }
        }
        ViewDecision::Commit(choice) => {
            if gpu.as_ref().is_some_and(|g| !g.idle()) {
                return;
            }
            let frame = target.as_ref().map(|t| t.image.clone());
            let layer_targets = layers.as_ref().map(|l| l.targets());
            let native_target = native.as_deref_mut();
            let resized = resize_resources(
                &mut state.cycle,
                &mut images,
                FrameTargets {
                    frame: frame.as_ref(),
                    layers: layer_targets.as_ref(),
                    native: native_target,
                },
                choice,
                window_size,
            );
            match resized {
                Ok(changed) => {
                    if changed {
                        state.last = None;
                        state.last_tags.clear();
                        state.last_ui.clear();
                    }
                    if let Err(e) = FrameSize::set_play_live(choice.frame) {
                        warn!(
                            "live view: frame {}x{} refused: {e}",
                            choice.frame.width, choice.frame.height
                        );
                        view.sync.send_failed();
                        return;
                    }
                    if changed {
                        if let Some(meshes) = meshes.as_deref_mut() {
                            let mesh = Rectangle::new(
                                choice.frame.width as f32,
                                choice.frame.height as f32,
                            );
                            let handle = meshes.add(mesh);
                            for mut mesh in &mut sprites {
                                mesh.0 = handle.clone();
                            }
                        }
                        commands.remove_resource::<ComposeJob>();
                        commands.remove_resource::<MotionJob>();
                    }
                    if let Some(ui) = ui.as_mut() {
                        if let Some(original) = ui.original.as_mut() {
                            original.resize_screen(Screen::play());
                        }
                    }
                    view.sync.committed();
                }
                Err(e) => {
                    warn!("live view: resize to {:?} failed: {e}", choice.frame);
                    view.sync.send_failed();
                }
            }
        }
    }
}

fn send_request(bridge: &mut BridgeResource, request: ViewRequest) -> bool {
    let msg = d2_proto::client::ViewChange {
        seq: request.seq,
        width: request.frame.width as u32,
        height: request.frame.height as u32,
    };
    match bridge.0.send(&msg) {
        Ok(_) => true,
        Err(e) => {
            warn!("live view: request {request:?} not sent: {e}");
            false
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::view_sync::ViewSync;
    use crate::ui::ScaleMode;

    fn desired(w: u32, h: u32, mode: ScaleMode) -> FrameSelection {
        FrameSelection::for_window(w, h, FrameSize::WIDE, mode)
    }

    #[test]
    fn ack_gates_every_commit_and_stale_answers_are_ignored() {
        let mut sync = ViewSync::default();
        let current = FrameSize::WIDE;
        let latest = desired(3840, 2160, ScaleMode::Integer);
        let request = match sync.poll(current, latest, None) {
            ViewDecision::Request(r) => r,
            other => panic!("expected request, got {other:?}"),
        };
        assert_eq!(sync.poll(current, latest, None), ViewDecision::Keep);
        assert_eq!(
            sync.poll(
                current,
                latest,
                Some(ViewRequest {
                    seq: 0,
                    frame: FrameSize::LOW
                })
            ),
            ViewDecision::Keep
        );
        assert_eq!(
            sync.poll(current, latest, Some(request)),
            ViewDecision::Commit(latest)
        );
        sync.committed();
        let tall = desired(800, 1600, ScaleMode::Integer);
        let request = match sync.poll(latest.frame, tall, Some(request)) {
            ViewDecision::Request(r) => r,
            other => panic!("expected request, got {other:?}"),
        };
        assert_eq!(
            sync.poll(latest.frame, tall, Some(request)),
            ViewDecision::Commit(tall)
        );
    }

    #[test]
    fn fit_returns_to_the_configured_normal_frame() {
        for (w, h) in [(1920, 1080), (2560, 1440), (800, 1600)] {
            let fit = FrameSelection::shipping(w, h, FrameSize::LOW, ScaleMode::Fit);
            assert_eq!(
                fit,
                FrameSelection {
                    frame: FrameSize::LOW,
                    mode: ScaleMode::Fit
                }
            );
        }
    }

    fn image(images: &mut Assets<Image>) -> Handle<Image> {
        let mut image = Image::new_target_texture(
            800,
            600,
            bevy::render::render_resource::TextureFormat::Rgba8Unorm,
            None,
        );
        image.data = None;
        images.add(image)
    }

    #[test]
    fn widest_and_tallest_transactions_wait_for_the_matching_ack() {
        let mut images = Assets::default();
        let frame = image(&mut images);
        let layers = SmoothLayers {
            world: image(&mut images),
            screen: image(&mut images),
            prev: image(&mut images),
        };
        let mut native = NativeUiTargets {
            cover: image(&mut images),
            add: image(&mut images),
            size: UVec2::new(800, 600),
        };
        let mut cycle = crate::scene::FrameCycle::new(800, 600).unwrap();
        let mut sync = ViewSync::default();
        let mut current = FrameSize::WIDE;
        for (w, h) in [
            (3840, 2160),
            (800, 1600),
            (1365, 767),
            (5120, 2160),
            (3440, 1440),
            (2560, 1080),
        ] {
            let desired = FrameSelection::for_window(w, h, FrameSize::WIDE, ScaleMode::Integer);
            let decision = sync.poll(current, desired, None);
            let ViewDecision::Request(request) = decision else {
                panic!("expected a request, got {decision:?}");
            };
            let stale = ViewRequest {
                seq: request.seq.wrapping_sub(1),
                frame: FrameSize::LOW,
            };
            assert_eq!(sync.poll(current, desired, Some(stale)), ViewDecision::Keep);
            assert_eq!(
                sync.poll(current, desired, Some(request)),
                ViewDecision::Commit(desired)
            );
            resize_resources(
                &mut cycle,
                &mut images,
                FrameTargets {
                    frame: Some(&frame),
                    layers: Some(&layers.targets()),
                    native: Some(&mut native),
                },
                desired,
                UVec2::new(w, h),
            )
            .unwrap();
            current = desired.frame;
            sync.committed();
            assert_eq!(
                (cycle.width(), cycle.height()),
                (desired.frame.width as u32, desired.frame.height as u32)
            );
        }
    }
}
