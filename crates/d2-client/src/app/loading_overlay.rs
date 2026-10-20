//! The loading screen over the running game: a black cover while
//! [`LoadingState`] is active, with frame `n` of `loadingscreen.dc6`
//! (Loading palette) at `placement` when the game's files are given. The
//! art sits on the 800 x 600 picture placed and scaled in the window as the
//! front end's is ([`art_rect`]), so on any window (4:3 or 16:9, any scale)
//! it shows where and as large as the menu before it.
//!
//! The cover is a UI node over the world, and the art an image node in it,
//! not drawn by the sprite compositor.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::ExtractedWindows;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems};
use d2_formats::dc6::Dc6;
use d2_formats::palette::Palette;

use crate::app::front_start::LoadingState;
use crate::assets::path::FileSource;
use crate::bridge::BridgeResource;
use crate::ui::front_end::screens::loading::{placement, Presented};

/// `play --dump-loading FILE`: every loading-screen present, one row each
/// (`index<TAB>kind<TAB>frame<TAB>x<TAB>y`, kind `loading`, `black` or
/// `world`), rewritten whenever a present is added. `// D2RUST's own`: the
/// presents come from [`LoadingScreen::presented`], the same list the
/// overlay shows; x, y are the cel position on the 800x600 display (L4 r3).
#[derive(Resource)]
pub struct LoadingDump(pub std::path::PathBuf, pub usize);

fn write_dump(path: &std::path::Path, presented: &[Presented]) {
    let (x, y) = placement(800, 600);
    let mut out = String::from("# loading-presents-1\tindex\tkind\tframe\tx\ty\n");
    for (i, p) in presented.iter().enumerate() {
        match p {
            Presented::Loading { frame } => {
                out.push_str(&format!("{i}\tloading\t{frame}\t{x}\t{y}\n"))
            }
            Presented::Black => out.push_str(&format!("{i}\tblack\t-\t-\t-\n")),
            Presented::World { act } => out.push_str(&format!("{i}\tworld\t{act}\t-\t-\n")),
        }
    }
    let _ = std::fs::write(path, out);
}

#[derive(Component)]
struct Cover;

#[derive(Component)]
struct Art;

/// Every art frame and its cel size.
#[derive(Resource, Default)]
struct LoadingFrames(Vec<(Handle<Image>, UVec2)>);

/// Where the loading art's cel shows in the window, as a UI node's
/// position and size in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArtRect {
    pub left: f32,
    pub top: f32,
    /// `None`: the cel's own size.
    pub size: Option<Vec2>,
}

/// Where a `cel` (width, height) cel of the art shows in a `window_w x
/// window_h` window (physical pixels, `scale_factor` physical per logical):
/// at its place on the 800 x 600 picture ([`placement`]: x and the bottom
/// row), with the picture placed and scaled as the front end's
/// ([`crate::app::front_host::front_presentation`]), its edges on the same
/// window pixels as the picture's. A window with no presentation (zero
/// size) keeps the cel at its own size, placed on the window's centre.
pub fn art_rect(window_w: u32, window_h: u32, scale_factor: f32, cel: UVec2) -> ArtRect {
    let sf = if scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let Ok(p) = crate::app::front_host::front_presentation(window_w, window_h) else {
        let (x, y) = placement((window_w as f32 / sf) as i32, (window_h as f32 / sf) as i32);
        return ArtRect {
            left: x as f32,
            top: (y + 1 - cel.y as i32) as f32,
            size: None,
        };
    };
    let (x, y) = placement(800, 600);
    let top = y + 1 - cel.y as i32;
    let (l, t) = p.from_frame(x, top);
    let (r, b) = p.from_frame(x + cel.x as i32, top + cel.y as i32);
    ArtRect {
        left: l as f32 / sf,
        top: t as f32 / sf,
        size: Some(Vec2::new((r - l) as f32 / sf, (b - t) as f32 / sf)),
    }
}

/// Puts the art node at `r`.
fn place_art(node: &mut Node, r: ArtRect) {
    node.left = Val::Px(r.left);
    node.top = Val::Px(r.top);
    let (w, h) = r
        .size
        .map_or((Val::Auto, Val::Auto), |s| (Val::Px(s.x), Val::Px(s.y)));
    node.width = w;
    node.height = h;
}

/// The [`art_rect`] of `cel` in `w`.
fn art_rect_in(w: &Window, cel: UVec2) -> ArtRect {
    art_rect(
        w.physical_width(),
        w.physical_height(),
        w.scale_factor(),
        cel,
    )
}

/// RGBA of every frame of the DC6 under `palette` (index 0 transparent).
pub fn frames_rgba(dc6: &Dc6, palette: &Palette) -> Vec<(u32, u32, Vec<u8>)> {
    dc6.frames
        .iter()
        .map(|f| {
            let mut px = Vec::with_capacity((f.width * f.height * 4) as usize);
            for &i in &f.pixels {
                let c = palette.colors[usize::from(i)];
                px.extend_from_slice(&[c.r, c.g, c.b, if i == 0 { 0 } else { 255 }]);
            }
            (f.width, f.height, px)
        })
        .collect()
}

/// The systems of the wiring below, once per app (`app::session`).
pub fn register_loading(app: &mut App) {
    app.add_systems(
        Update,
        (
            spawn_cover.run_if(resource_added::<LoadingState>),
            (track, dump_presents)
                .chain()
                .run_if(resource_exists::<LoadingState>)
                .run_if(resource_exists::<BridgeResource>),
        )
            .chain(),
    );
    add_present_feedback(app);
}

/// Installs the loading screen. `files`: the game's archives (art and the
/// Loading palette); without them the cover is plain black.
pub fn add_loading(app: &mut impl crate::app::session::Setup, files: Option<Arc<dyn FileSource>>) {
    app.register("loading", register_loading);
    let frames = files
        .and_then(|s| {
            // L3 r1: the path names a DC6 file (`.dc6` appended by the
            // cel loader).
            let art = format!(
                "{}.dc6",
                crate::ui::front_end::screens::loading::art_path(0)
            );
            let dc6 = crate::assets::path::read_dc6(&*s, &art)?.ok()?;
            let pal =
                crate::assets::path::read_palette(&*s, r"data\global\palette\Loading\pal.dat")?
                    .ok()?;
            Some(frames_rgba(&dc6, &pal))
        })
        .unwrap_or_default();
    let w = app.world_mut();
    w.insert_resource(LoadingState::default());
    w.insert_resource(PendingFrames(frames));
    w.insert_resource(LoadingFrames::default());
    w.insert_resource(LoadingPass {
        shown: crate::launch::env_os("D2RUST_SHOWN_FILE").map(std::path::PathBuf::from),
        ..default()
    });
    // The window's clear before the cover is drawn is black.
    w.insert_resource(ClearColor(Color::BLACK));
    if let Some(f) = w.get_resource::<PresentFeedback>() {
        // A new game counts its passes from 1 again.
        f.0.store(0, Ordering::Release);
    }
}

/// Which client pass the window last presented. `// D2RUST's own`:
/// the game's loading draw ends with a synchronous `EndScene` present,
/// so each loading frame is on screen before the next pass runs; Bevy
/// renders a pass later, on its render world, and this count lets the
/// loading model wait for it. Absent when there is no renderer (tests).
#[derive(Resource, Clone, Default)]
pub struct PresentFeedback(Arc<AtomicU32>);

impl PresentFeedback {
    /// The latest client pass (1-based) whose frame the window presented;
    /// 0 before the first present.
    pub fn presented(&self) -> u32 {
        self.0.load(Ordering::Acquire)
    }
}

/// The loading pass count and the pass whose present the model waits on.
#[derive(Resource, Default)]
struct LoadingPass {
    /// Client passes run by [`track`] (1-based once it has run).
    pass: u32,
    /// The pass that added the last present.
    wait_for: u32,
    /// Presents shown so far (the index of the next one to show).
    seen: usize,
    /// Launcher hand-off (`D2RUST_SHOWN_FILE`): the file the launcher writes
    /// once the menu window, which covers this one until then, is gone.
    /// Taken when it exists.
    shown: Option<std::path::PathBuf>,
}

/// Render world: the client pass being rendered, and the one acquired a
/// window texture for (presented at the end of the render step).
#[derive(Resource, Default)]
struct ExtractedPass {
    pass: u32,
    pending: Option<u32>,
}

fn add_present_feedback(app: &mut App) {
    let feedback = PresentFeedback::default();
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render
        .insert_resource(feedback.clone())
        .init_resource::<ExtractedPass>()
        .add_systems(ExtractSchedule, extract_pass)
        .add_systems(
            Render,
            (
                note_acquired.in_set(RenderSystems::Prepare),
                note_presented.in_set(RenderSystems::Cleanup),
            ),
        );
    app.insert_resource(feedback);
}

fn extract_pass(mut ex: ResMut<ExtractedPass>, main: Extract<Option<Res<LoadingPass>>>) {
    ex.pass = main.as_ref().map_or(0, |p| p.pass);
}

/// After the window texture is acquired: this pass will be presented.
fn note_acquired(mut ex: ResMut<ExtractedPass>, windows: Res<ExtractedWindows>) {
    let acquired = windows
        .primary
        .and_then(|e| windows.windows.get(&e))
        .is_some_and(|w| w.swap_chain_texture.is_some());
    ex.pending = acquired.then_some(ex.pass);
}

/// After the render step: the texture was handed to the presenter.
fn note_presented(
    mut ex: ResMut<ExtractedPass>,
    windows: Res<ExtractedWindows>,
    feedback: Res<PresentFeedback>,
) {
    let Some(pass) = ex.pending.take() else {
        return;
    };
    let presented = windows
        .primary
        .and_then(|e| windows.windows.get(&e))
        .is_some_and(|w| w.swap_chain_texture.is_none());
    if presented {
        feedback.0.fetch_max(pass, Ordering::AcqRel);
    }
}

#[derive(Resource)]
struct PendingFrames(Vec<(u32, u32, Vec<u8>)>);

fn spawn_cover(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut pending: ResMut<PendingFrames>,
    mut frames: ResMut<LoadingFrames>,
    state: Res<LoadingState>,
    windows: Query<&Window>,
) {
    for (w, h, px) in pending.0.drain(..) {
        let mut img = Image::new(
            Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            px,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        img.sampler = bevy::image::ImageSampler::nearest();
        frames.0.push((images.add(img), UVec2::new(w, h)));
    }
    commands
        .spawn((
            Cover,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::BLACK),
            GlobalZIndex(1000),
            crate::app::session::GameEntity,
        ))
        .with_children(|p| {
            // The game-start draw is on screen from the first presented
            // frame: the first passes build the game and can take
            // seconds, and frame 0 shows for that time.
            let first = match (state.screen.presented.last(), windows.single()) {
                (Some(Presented::Loading { frame }), Ok(w)) => frames
                    .0
                    .get(*frame as usize)
                    .map(|(h, cel)| (h.clone(), art_rect_in(w, *cel))),
                _ => None,
            };
            match first {
                Some((h, r)) => {
                    let mut node = Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    };
                    place_art(&mut node, r);
                    p.spawn((Art, node, ImageNode::new(h), Visibility::Inherited))
                }
                None => p.spawn((
                    Art,
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                    Visibility::Hidden,
                )),
            };
        });
}

/// Writes [`LoadingDump`] when a present was added (after `track`).
fn dump_presents(state: Res<LoadingState>, dump: Option<ResMut<LoadingDump>>) {
    if let Some(mut d) = dump {
        let n = state.screen.presented.len();
        if n != d.1 {
            write_dump(&d.0, &state.screen.presented);
            d.1 = n;
        }
    }
}

/// One client pass: feed the model state, show or hide the cover, set the
/// art frame. A pass that added a loading or black present holds the
/// model until the window has presented it (L6 r4); a window of zero size
/// (minimized) presents nothing and is not waited on. Started by the
/// launcher, the model also waits until the menu window is gone.
#[allow(clippy::too_many_arguments)]
fn track(
    mut state: ResMut<LoadingState>,
    mut pass: ResMut<LoadingPass>,
    feedback: Option<Res<PresentFeedback>>,
    bridge: Res<BridgeResource>,
    frames: Res<LoadingFrames>,
    windows: Query<&Window>,
    mut cover: Query<&mut Visibility, (With<Cover>, Without<Art>)>,
    mut art: Query<(&mut Visibility, &mut Node, Option<&mut ImageNode>, Entity), With<Art>>,
    mut commands: Commands,
) {
    pass.pass += 1;
    // The window is under the menu's until the hand-off: frame 0 stays.
    if let Some(f) = &pass.shown {
        if !f.exists() {
            return;
        }
        pass.shown = None;
    }
    let shown = windows
        .single()
        .is_ok_and(|w| w.physical_width() > 0 && w.physical_height() > 0);
    if let Some(f) = feedback.filter(|_| shown) {
        if f.presented() < pass.wait_for {
            return;
        }
    }
    // Every recorded present is shown in its own pass, in order (L4 r5:
    // each loading draw is a present); the model runs again once all are.
    if pass.seen >= state.screen.presented.len() {
        state.frame(bridge.0.world());
    }
    let Some(&next) = state.screen.presented.get(pass.seen) else {
        return;
    };
    pass.seen += 1;
    // Only loading-screen presents are waited on; world frames run at the
    // renderer's pace.
    if !matches!(next, Presented::World { .. }) {
        pass.wait_for = pass.pass;
    }
    let next = match next {
        // Black, not the art, after an act video.
        Presented::Loading { .. } if state.screen.after_video => Presented::Black,
        p => p,
    };
    let covering = !matches!(next, Presented::World { .. });
    for mut v in &mut cover {
        *v = if covering {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    let Presented::Loading { frame } = next else {
        for (mut v, ..) in &mut art {
            *v = Visibility::Hidden;
        }
        return;
    };
    let Ok(w) = windows.single() else { return };
    let Some((h, cel)) = frames.0.get(frame as usize) else {
        return;
    };
    // The cel's place is (x, bottom y): its rows end on row y.
    let r = art_rect_in(w, *cel);
    for (mut v, mut node, img, e) in &mut art {
        place_art(&mut node, r);
        *v = Visibility::Inherited;
        match img {
            Some(mut i) => i.image = h.clone(),
            None => {
                commands.entity(e).insert(ImageNode::new(h.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CEL: UVec2 = UVec2::new(256, 256);

    #[test]
    fn at_800_x_600_the_art_is_where_it_always_was() {
        // (272, 428) bottom-left: rows 173..= 428, at the cel's size.
        let r = art_rect(800, 600, 1.0, CEL);
        assert_eq!((r.left, r.top), (272.0, 173.0));
        assert_eq!(r.size, Some(Vec2::new(256.0, 256.0)));
    }

    #[test]
    fn the_art_scales_and_sits_like_the_menu_at_16_9_and_4_3() {
        crate::ui::set_scale_mode(crate::ui::ScaleMode::Fit);
        for (w, h) in [
            (1920, 1080),
            (2560, 1440),
            (3840, 2160),
            (1600, 1200),
            (1024, 768),
        ] {
            let p = crate::app::front_host::front_presentation(w, h).unwrap();
            let r = art_rect(w, h, 1.0, CEL);
            // Its edges are the picture's pixels 272 and 528 across, 173
            // and 429 down.
            let (l, t) = p.from_frame(272, 173);
            let (rt, b) = p.from_frame(528, 429);
            assert_eq!((r.left, r.top), (l as f32, t as f32), "{w}x{h}");
            assert_eq!(
                r.size,
                Some(Vec2::new((rt - l) as f32, (b - t) as f32)),
                "{w}x{h}"
            );
            // Centred on the window as on the picture: within one picture pixel
            // and a rounding pixel (its rows 173..= 428 centre half a row low).
            let s = r.size.unwrap();
            assert!(
                (r.left + s.x / 2.0 - w as f32 / 2.0).abs() <= p.sx + 1.0,
                "{w}x{h}"
            );
            assert!(
                (r.top + s.y / 2.0 - h as f32 / 2.0).abs() <= p.sy + 1.0,
                "{w}x{h}"
            );
            // As large as the menu: the picture's scale.
            assert!((s.y - 256.0 * p.sy).abs() <= 1.0, "{w}x{h}");
        }
        // 1080p: the picture is 1440 x 1080 from x 240, scale 1.8; picture
        // column 272 starts on window column 730 (240 + 489.6, rounded up).
        let r = art_rect(1920, 1080, 1.0, CEL);
        assert_eq!(r.left, 730.0);
    }

    #[test]
    fn display_scaling_gives_logical_pixels() {
        let a = art_rect(1920, 1080, 1.0, CEL);
        let b = art_rect(1920, 1080, 1.5, CEL);
        assert_eq!(b.left, a.left / 1.5);
        assert_eq!(b.size.unwrap(), a.size.unwrap() / 1.5);
    }

    #[test]
    fn a_window_with_no_presentation_keeps_the_cel_at_its_size() {
        let r = art_rect(0, 0, 1.0, CEL);
        assert_eq!(r.size, None);
    }
}
