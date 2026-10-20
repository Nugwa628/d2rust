//! The Bevy app: loads a DS1 map through the `mpq://` asset source and draws
//! it with the palette material. Two modes:
//! - `View`: a window with pan (arrows/WASD) and zoom (mouse wheel).
//! - `Verify`: headless; renders a view to an offscreen image, captures it,
//!   and compares it byte-for-byte with the CPU reference renderer.
//!
//! [`play`] is the client proper: a window running the local
//! single-player game ([`single_player`], on a server thread,
//! [`server_thread`]) through the bridge and the world view.

pub mod anim_names;
pub mod audio_dump;
pub mod automap;
pub mod autoplay_front;
pub mod autoplay_host;
pub mod checks;
pub mod client_state;
pub mod config;
pub mod control;
pub mod coverage;
pub mod death;
pub mod dev;
pub mod explore;
pub mod frame_limit;
pub mod front_cels;
pub mod front_fill;
pub mod front_host;
pub mod front_start;
pub mod front_ui;
pub mod game_map;
pub mod hardcore;
pub mod hire_stats;
pub mod hud;
pub mod items;
pub mod loading_overlay;
pub mod missile_art;
pub use d2_server::monster_ai;
pub use d2_server::monster_drop;
pub use d2_server::npc_seams;
#[cfg(feature = "online")]
pub mod online;
#[cfg(feature = "online")]
pub mod online_prices;
pub mod packet_dump;
pub mod palette;
pub mod perf;
pub mod perf_summary;
pub mod play;
pub mod play_start;
pub mod poke;
pub mod registry;
pub mod repro;
pub use d2_server::rest;
pub mod rng_dump;
pub mod save;
pub mod save_full;
pub mod save_gaps;
pub mod send;
pub mod server_thread;
pub mod session;
pub mod shop_prices;
pub mod single_player;
mod skill_rows;
/// Server skill seams with the client bridge's skill-row projection.
pub mod skill_rest {
    pub use super::skill_rows::skill_row;
    pub use d2_server::skill_rest::*;
}
pub mod soak;
pub mod sound;
pub mod state_dump;
pub mod strings;
pub mod sweep;
pub mod ui;
pub mod visibility;
pub mod vsync;
pub use d2_server::weapons;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::asset::{AssetMetaCheck, LoadState};
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::system::SystemParam;
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::view::Msaa;
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use d2_formats::mpq::ArchiveSet;

use crate::assets::{
    asset_path, D2AssetsPlugin, Dc6Asset, DccAsset, Ds1Asset, Dt1Asset, MpqSourcePlugin,
    PaletteAsset,
};
use crate::map::{self, cpu, Layout, TileLibrary};
use crate::render::{index_image, palette_image, PaletteMaterial, PaletteRenderPlugin};

/// Frames to wait after spawning before the first capture (pipelines compile
/// and textures upload asynchronously).
const WARMUP_FRAMES: u32 = 60;
/// Frames between capture attempts.
const RETRY_FRAMES: u32 = 30;
/// Capture attempts before giving up. A capture is only judged once it is
/// identical to the previous one (the render has stopped changing).
const MAX_ATTEMPTS: u32 = 10;
/// Give up if the map isn't spawned by then (missing assets, hangs).
const TIMEOUT_FRAMES: u32 = 3000;

pub struct VerifyConfig {
    pub view: cpu::View,
    /// Expected RGBA bytes from the CPU reference renderer.
    pub reference: Vec<u8>,
    pub out_dir: PathBuf,
}

pub enum Mode {
    /// Interactive window; `exit_after` frames (if set) closes it, for smoke
    /// tests.
    View {
        exit_after: Option<u32>,
    },
    Verify(VerifyConfig),
}

#[derive(Resource)]
struct ExitAfter {
    frames: u32,
    seen: u32,
}

/// Where the view smoke test saves its window screenshot (gitignored).
pub const VIEW_SMOKE_SCREENSHOT: &str = "game/renders/view-smoke.png";

fn exit_after_frames(
    mut commands: Commands,
    mut exit_after: ResMut<ExitAfter>,
    spawned: Option<Res<SpawnedMap>>,
    mut exit: MessageWriter<AppExit>,
) {
    exit_after.seen += 1;
    // Capture what the window shows a little before closing.
    if exit_after.seen + 30 == exit_after.frames {
        commands.spawn(Screenshot::primary_window()).observe(
            bevy::render::view::screenshot::save_to_disk(VIEW_SMOKE_SCREENSHOT),
        );
    }
    if exit_after.seen >= exit_after.frames {
        if spawned.is_some() {
            info!(
                "smoke test: map shown, exiting after {} frames",
                exit_after.seen
            );
            exit.write(AppExit::Success);
        } else {
            error!(
                "smoke test: map not spawned after {} frames",
                exit_after.seen
            );
            exit.write(AppExit::error());
        }
    }
}

pub struct MapConfig {
    pub ds1_path: String,
    pub wall_base: i32,
}

#[derive(Resource)]
struct MapSettings {
    ds1_path: String,
    wall_base: i32,
}

#[derive(Default)]
enum LoadPhase {
    #[default]
    Start,
    Ds1(Handle<Ds1Asset>),
    Tiles {
        ds1: Handle<Ds1Asset>,
        dt1s: Vec<(String, Handle<Dt1Asset>)>,
        palette: Handle<PaletteAsset>,
        dcc: Handle<DccAsset>,
        dc6: Handle<Dc6Asset>,
    },
    Spawned,
}

#[derive(Resource, Default)]
struct MapLoad {
    phase: LoadPhase,
}

/// Set once the map's entities exist; holds what the verifier needs.
#[derive(Resource)]
struct SpawnedMap {
    bounds: map::Bounds,
}

#[derive(Resource)]
struct Verify {
    config: VerifyConfig,
    target: Handle<Image>,
    frames: u32,
    spawned_at: Option<u32>,
    next_attempt: Option<u32>,
    attempts: u32,
    in_flight: bool,
    /// Hash of the previous capture, to detect a stable render.
    last_capture: Option<u64>,
}

#[derive(Component)]
struct MainCamera;

pub fn run(archives: Arc<ArchiveSet>, map_config: MapConfig, mode: Mode) -> AppExit {
    let mut app = App::new();
    app.add_plugins(MpqSourcePlugin { archives });
    let asset_plugin = AssetPlugin {
        meta_check: AssetMetaCheck::Never,
        ..default()
    };
    match &mode {
        Mode::View { .. } => {
            app.add_plugins(crate::mute::plugins(DefaultPlugins).set(asset_plugin).set(
                WindowPlugin {
                    primary_window: Some(Window {
                        title: "D2RUST map preview".into(),
                        ..default()
                    }),
                    ..default()
                },
            ));
        }
        Mode::Verify(_) => {
            // Headless: never an audio device (`crate::mute`).
            app.add_plugins(
                DefaultPlugins
                    .set(asset_plugin)
                    .set(WindowPlugin {
                        primary_window: None,
                        exit_condition: ExitCondition::DontExit,
                        ..default()
                    })
                    .disable::<WinitPlugin>()
                    .disable::<bevy::audio::AudioPlugin>(),
            )
            .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / 60.0,
            )));
        }
    }
    app.add_plugins((D2AssetsPlugin, PaletteRenderPlugin))
        .insert_resource(MapSettings {
            ds1_path: map_config.ds1_path,
            wall_base: map_config.wall_base,
        })
        .init_resource::<MapLoad>()
        .add_systems(Update, drive_map_load);

    match mode {
        Mode::View { exit_after } => {
            // Inert until a bridge and a world view state are inserted.
            app.add_plugins((
                crate::bridge::BridgePlugin,
                crate::world_view::WorldViewPlugin::default(),
            ));
            app.add_systems(Startup, spawn_window_camera)
                .add_systems(Update, (camera_controls, center_camera_on_spawn));
            if let Some(frames) = exit_after {
                app.insert_resource(ExitAfter { frames, seen: 0 })
                    .add_systems(Update, exit_after_frames);
            }
        }
        Mode::Verify(config) => {
            let (w, h) = (config.view.width, config.view.height);
            let target = {
                let mut images = app.world_mut().resource_mut::<Assets<Image>>();
                images.add(Image::new_target_texture(
                    w,
                    h,
                    TextureFormat::Rgba8UnormSrgb,
                    None,
                ))
            };
            let view = config.view;
            app.insert_resource(Verify {
                config,
                target: target.clone(),
                frames: 0,
                spawned_at: None,
                next_attempt: None,
                attempts: 0,
                in_flight: false,
                last_capture: None,
            })
            .add_systems(Startup, move |mut commands: Commands| {
                // Camera center = view center; with even sizes, world
                // integers fall on pixel boundaries (1 unit = 1 pixel).
                let cx = view.left as f32 + view.width as f32 / 2.0;
                let cy = -(view.top as f32 + view.height as f32 / 2.0);
                commands.spawn((
                    Camera2d,
                    Camera {
                        clear_color: ClearColorConfig::Custom(Color::srgb_u8(0, 0, 0)),
                        ..default()
                    },
                    RenderTarget::Image(target.clone().into()),
                    Msaa::Off,
                    Tonemapping::None,
                    Transform::from_xyz(cx, cy, 0.0),
                    MainCamera,
                ));
            })
            .add_systems(Update, verify_driver);
        }
    }
    app.run()
}

/// The loaded D2 asset collections the map loader reads.
#[derive(SystemParam)]
struct D2Assets<'w> {
    ds1s: Res<'w, Assets<Ds1Asset>>,
    dt1s: Res<'w, Assets<Dt1Asset>>,
    palettes: Res<'w, Assets<PaletteAsset>>,
    dccs: Res<'w, Assets<DccAsset>>,
    dc6s: Res<'w, Assets<Dc6Asset>>,
}

/// The render asset collections the map spawner writes.
#[derive(SystemParam)]
struct MapRenderAssets<'w> {
    images: ResMut<'w, Assets<Image>>,
    materials: ResMut<'w, Assets<PaletteMaterial>>,
    meshes: ResMut<'w, Assets<Mesh>>,
}

fn drive_map_load(
    mut commands: Commands,
    settings: Res<MapSettings>,
    mut load: ResMut<MapLoad>,
    server: Res<AssetServer>,
    d2: D2Assets,
    mut gpu: MapRenderAssets,
    mut exit: MessageWriter<AppExit>,
) {
    let phase = std::mem::take(&mut load.phase);
    load.phase = match phase {
        LoadPhase::Start => LoadPhase::Ds1(server.load(asset_path(&settings.ds1_path))),
        LoadPhase::Ds1(handle) => match d2.ds1s.get(&handle) {
            Some(ds1) => {
                let dt1s = map::tiles::dt1_paths(&ds1.0)
                    .into_iter()
                    .map(|p| {
                        let h = server.load(asset_path(&p));
                        (p, h)
                    })
                    .collect();
                let palette = server.load(asset_path(&map::palette_path(ds1.0.act)));
                LoadPhase::Tiles {
                    ds1: handle,
                    dt1s,
                    palette,
                    dcc: server.load(asset_path(map::DEMO_DCC)),
                    dc6: server.load(asset_path(map::DEMO_DC6)),
                }
            }
            None => {
                if let LoadState::Failed(err) = server.load_state(&handle) {
                    error!("loading {}: {err}", settings.ds1_path);
                    exit.write(AppExit::error());
                }
                LoadPhase::Ds1(handle)
            }
        },
        LoadPhase::Tiles {
            ds1,
            dt1s: tiles,
            palette,
            dcc,
            dc6,
        } => {
            let settled = |h: &Handle<Dt1Asset>| {
                d2.dt1s.contains(h) || matches!(server.load_state(h), LoadState::Failed(_))
            };
            for (what, state) in [
                ("palette", server.load_state(&palette)),
                (map::DEMO_DCC, server.load_state(&dcc)),
                (map::DEMO_DC6, server.load_state(&dc6)),
            ] {
                if let LoadState::Failed(err) = state {
                    error!("loading {what}: {err}");
                    exit.write(AppExit::error());
                }
            }
            match (
                tiles.iter().all(|(_, h)| settled(h)),
                d2.palettes.get(&palette),
                d2.dccs.get(&dcc),
                d2.dc6s.get(&dc6),
            ) {
                (true, Some(pal), Some(dcc_asset), Some(dc6_asset)) => {
                    let ds1_asset = d2.ds1s.get(&ds1).expect("loaded above");
                    let mut library = TileLibrary::new();
                    for (path, h) in &tiles {
                        match d2.dt1s.get(h) {
                            Some(dt1) => library.add_dt1(&dt1.0),
                            None => warn!("tile file not loaded: {path}"),
                        }
                    }
                    let mut layout = map::build(&ds1_asset.0, &library, settings.wall_base);
                    map::add_sprites(
                        &mut layout,
                        &mut library,
                        &ds1_asset.0,
                        &dcc_asset.0,
                        &dc6_asset.0,
                    );
                    let missing: usize = layout.missing.values().sum();
                    info!(
                        "map {}: {} draw items, {} cells with missing tiles",
                        settings.ds1_path,
                        layout.items.len(),
                        missing
                    );
                    if let Some(bounds) = layout.bounds {
                        spawn_map(
                            &mut commands,
                            &layout,
                            &library,
                            &pal.0,
                            &mut gpu.images,
                            &mut gpu.materials,
                            &mut gpu.meshes,
                        );
                        commands.insert_resource(SpawnedMap { bounds });
                    } else {
                        error!("map has nothing to draw");
                        exit.write(AppExit::error());
                    }
                    LoadPhase::Spawned
                }
                _ => LoadPhase::Tiles {
                    ds1,
                    dt1s: tiles,
                    palette,
                    dcc,
                    dc6,
                },
            }
        }
        LoadPhase::Spawned => LoadPhase::Spawned,
    };
}

/// Depth range used for draw order (camera near/far is ±1000).
const Z_MIN: f32 = -900.0;
const Z_RANGE: f32 = 1800.0;

/// z for the `i`-th of `n` items: strictly increasing, back to front.
pub fn item_z(i: usize, n: usize) -> f32 {
    let step = (Z_RANGE / n.max(1) as f32).min(0.25);
    Z_MIN + i as f32 * step
}

fn spawn_map(
    commands: &mut Commands,
    layout: &Layout,
    library: &TileLibrary,
    palette: &d2_formats::palette::Palette,
    images: &mut Assets<Image>,
    materials: &mut Assets<PaletteMaterial>,
    meshes: &mut Assets<Mesh>,
) {
    let palette = images.add(palette_image(palette));
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    let mut tile_materials: Vec<Option<Handle<PaletteMaterial>>> = vec![None; library.images.len()];
    let n = layout.items.len();
    for (i, item) in layout.items.iter().enumerate() {
        let img = &library.images[item.image];
        let material = tile_materials[item.image]
            .get_or_insert_with(|| {
                let indices = images.add(index_image(img.width, img.height, img.pixels.clone()));
                materials.add(PaletteMaterial {
                    indices,
                    palette: palette.clone(),
                })
            })
            .clone();
        let (w, h) = (img.width as f32, img.height as f32);
        commands.spawn((
            Mesh2d(quad.clone()),
            MeshMaterial2d(material),
            Transform::from_xyz(
                item.x as f32 + w / 2.0,
                -(item.y as f32 + h / 2.0),
                item_z(i, n),
            )
            .with_scale(Vec3::new(w, h, 1.0)),
        ));
    }
}

fn spawn_window_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::srgb_u8(0, 0, 0)),
            ..default()
        },
        Msaa::Off,
        Tonemapping::None,
        MainCamera,
    ));
}

fn center_camera_on_spawn(
    spawned: Option<Res<SpawnedMap>>,
    mut done: Local<bool>,
    mut camera: Query<&mut Transform, With<MainCamera>>,
) {
    if *done {
        return;
    }
    let Some(spawned) = spawned else { return };
    let b = spawned.bounds;
    if let Ok(mut t) = camera.single_mut() {
        t.translation.x = ((b.x0 + b.x1) / 2) as f32;
        t.translation.y = -(((b.y0 + b.y1) / 2) as f32);
        *done = true;
    }
}

fn camera_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mut wheel: MessageReader<MouseWheel>,
    time: Res<Time>,
    mut camera: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    let Ok((mut transform, mut projection)) = camera.single_mut() else {
        return;
    };
    let Projection::Orthographic(ortho) = &mut *projection else {
        return;
    };
    let mut dir = Vec2::ZERO;
    if keys.any_pressed([KeyCode::ArrowLeft, KeyCode::KeyA]) {
        dir.x -= 1.0;
    }
    if keys.any_pressed([KeyCode::ArrowRight, KeyCode::KeyD]) {
        dir.x += 1.0;
    }
    if keys.any_pressed([KeyCode::ArrowUp, KeyCode::KeyW]) {
        dir.y += 1.0;
    }
    if keys.any_pressed([KeyCode::ArrowDown, KeyCode::KeyS]) {
        dir.y -= 1.0;
    }
    let speed = 800.0 * ortho.scale * time.delta_secs();
    transform.translation += (dir * speed).extend(0.0);
    for ev in wheel.read() {
        let factor = if ev.y > 0.0 { 0.8 } else { 1.25 };
        ortho.scale = (ortho.scale * factor).clamp(0.25, 16.0);
    }
}

fn verify_driver(
    mut commands: Commands,
    mut verify: ResMut<Verify>,
    spawned: Option<Res<SpawnedMap>>,
    mut exit: MessageWriter<AppExit>,
) {
    verify.frames += 1;
    let frame = verify.frames;
    if spawned.is_none() {
        if frame > TIMEOUT_FRAMES {
            error!("timed out after {TIMEOUT_FRAMES} frames waiting for the map to load");
            exit.write(AppExit::error());
        }
        return;
    }
    let spawned_at = *verify.spawned_at.get_or_insert(frame);
    let next = *verify
        .next_attempt
        .get_or_insert(spawned_at + WARMUP_FRAMES);
    if verify.in_flight || frame < next {
        return;
    }
    verify.in_flight = true;
    verify.attempts += 1;
    info!("capture attempt {}/{MAX_ATTEMPTS}", verify.attempts);
    commands
        .spawn(Screenshot::image(verify.target.clone()))
        .observe(on_captured);
}

/// What to do after a capture has been compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    /// Mismatch, but the render is still changing: capture again.
    Retry,
    /// Mismatch on a stable render (identical to the previous capture).
    Fail,
    /// Still changing after the last allowed attempt.
    Unstable,
}

/// Decides the outcome of one capture.
pub fn verdict(report: &CompareReport, stable: bool, attempt: u32, max_attempts: u32) -> Verdict {
    match (report.mismatched == 0 && !report.unrendered, stable) {
        (true, _) => Verdict::Pass,
        (false, true) if !report.unrendered => Verdict::Fail,
        _ if attempt >= max_attempts => Verdict::Unstable,
        _ => Verdict::Retry,
    }
}

fn hash_image(image: &Image) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    image.data.hash(&mut h);
    h.finish()
}

fn on_captured(
    captured: On<ScreenshotCaptured>,
    mut verify: ResMut<Verify>,
    mut exit: MessageWriter<AppExit>,
) {
    verify.in_flight = false;
    let hash = hash_image(&captured.image);
    let stable = verify.last_capture == Some(hash);
    verify.last_capture = Some(hash);
    let report = match compare_capture(&captured.image, &verify.config) {
        Ok(r) => r,
        Err(e) => {
            error!("could not compare capture: {e}");
            exit.write(AppExit::error());
            return;
        }
    };
    match verdict(&report, stable, verify.attempts, MAX_ATTEMPTS) {
        Verdict::Pass => {
            info!("GPU output matches the CPU reference exactly: {report}");
            exit.write(AppExit::Success);
        }
        Verdict::Retry => {
            warn!(
                "attempt {}: {report}; render still changing, capturing again",
                verify.attempts
            );
            verify.next_attempt = Some(verify.frames + RETRY_FRAMES);
        }
        Verdict::Fail => {
            error!("GPU output differs from the CPU reference (stable render): {report}");
            exit.write(AppExit::error());
        }
        Verdict::Unstable => {
            error!("render never stabilized in {MAX_ATTEMPTS} attempts; last: {report}");
            exit.write(AppExit::error());
        }
    }
}

/// Pixel comparison result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompareReport {
    pub pixels: usize,
    pub mismatched: usize,
    pub max_channel_diff: u8,
    /// Bounding box of mismatches `(x0, y0, x1, y1)`, inclusive.
    pub bbox: Option<(u32, u32, u32, u32)>,
    /// True if every captured pixel has alpha 0: nothing was rendered into
    /// the target yet (the camera always clears to opaque black).
    pub unrendered: bool,
}

impl std::fmt::Display for CompareReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.unrendered {
            return write!(f, "capture is empty (renderer not ready yet)");
        }
        write!(
            f,
            "{} of {} pixels differ, max channel diff {}",
            self.mismatched, self.pixels, self.max_channel_diff
        )?;
        if let Some((x0, y0, x1, y1)) = self.bbox {
            write!(f, ", within ({x0},{y0})-({x1},{y1})")?;
        }
        Ok(())
    }
}

/// The RGBA pixels of a buffer.
fn rgba_pixels(buf: &[u8]) -> std::slice::Iter<'_, [u8; 4]> {
    buf.as_chunks::<4>().0.iter()
}

/// Compares two RGBA8 buffers of `width` pixels per row.
pub fn compare_rgba(actual: &[u8], expected: &[u8], width: u32) -> CompareReport {
    let mut report = CompareReport {
        pixels: expected.len() / 4,
        mismatched: 0,
        max_channel_diff: 0,
        bbox: None,
        unrendered: !actual.is_empty() && rgba_pixels(actual).all(|p| p[3] == 0),
    };
    for (i, (a, e)) in rgba_pixels(actual).zip(rgba_pixels(expected)).enumerate() {
        let diff = a
            .iter()
            .zip(e)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        if diff > 0 {
            report.mismatched += 1;
            report.max_channel_diff = report.max_channel_diff.max(diff);
            let (x, y) = ((i as u32) % width, (i as u32) / width);
            report.bbox = Some(match report.bbox {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    report
}

fn compare_capture(image: &Image, config: &VerifyConfig) -> anyhow::Result<CompareReport> {
    let rgba = image
        .clone()
        .try_into_dynamic()
        .map_err(|e| anyhow::anyhow!("capture format: {e:?}"))?
        .to_rgba8();
    let (w, h) = rgba.dimensions();
    anyhow::ensure!(
        (w, h) == (config.view.width, config.view.height),
        "capture is {w}x{h}, expected {}x{}",
        config.view.width,
        config.view.height
    );
    let report = compare_rgba(rgba.as_raw(), &config.reference, w);
    std::fs::create_dir_all(&config.out_dir)?;
    rgba.save(config.out_dir.join("gpu.png"))?;
    image::save_buffer(
        config.out_dir.join("cpu.png"),
        &config.reference,
        w,
        h,
        image::ExtendedColorType::Rgba8,
    )?;
    if report.mismatched > 0 {
        // Red where pixels differ, the CPU image darkened elsewhere.
        let mut diff = Vec::with_capacity(config.reference.len());
        for (a, e) in rgba_pixels(rgba.as_raw()).zip(rgba_pixels(&config.reference)) {
            if a == e {
                diff.extend_from_slice(&[e[0] / 3, e[1] / 3, e[2] / 3, 255]);
            } else {
                diff.extend_from_slice(&[255, 0, 0, 255]);
            }
        }
        image::save_buffer(
            config.out_dir.join("diff.png"),
            &diff,
            w,
            h,
            image::ExtendedColorType::Rgba8,
        )?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn z_is_strictly_increasing_within_range() {
        for n in [1, 10, 7000, 100_000] {
            let z: Vec<f32> = (0..n).map(|i| item_z(i, n)).collect();
            assert!(z.windows(2).all(|w| w[1] > w[0]), "n = {n}");
            assert!(
                z[0] >= Z_MIN && z[n - 1] < Z_MIN + Z_RANGE + 0.01,
                "n = {n}"
            );
        }
    }

    #[test]
    fn compare_reports_differences() {
        let expected = [0u8, 0, 0, 255, 10, 20, 30, 255, 1, 1, 1, 255, 0, 0, 0, 255];
        let mut actual = expected;
        assert_eq!(compare_rgba(&actual, &expected, 2).mismatched, 0);
        actual[5] = 23; // pixel (1,0) green +3
        actual[12] = 9; // pixel (1,1) red +9
        let r = compare_rgba(&actual, &expected, 2);
        assert_eq!(r.mismatched, 2);
        assert_eq!(r.max_channel_diff, 9);
        assert_eq!(r.bbox, Some((1, 0, 1, 1)));
        assert!(!r.unrendered);
    }

    #[test]
    fn verdicts() {
        let ok = CompareReport {
            pixels: 10,
            mismatched: 0,
            max_channel_diff: 0,
            bbox: None,
            unrendered: false,
        };
        let bad = CompareReport {
            mismatched: 3,
            max_channel_diff: 5,
            bbox: Some((0, 0, 1, 1)),
            ..ok
        };
        let empty = CompareReport {
            mismatched: 10,
            unrendered: true,
            ..bad
        };
        assert_eq!(verdict(&ok, false, 1, 10), Verdict::Pass);
        assert_eq!(
            verdict(&bad, false, 1, 10),
            Verdict::Retry,
            "still changing"
        );
        assert_eq!(verdict(&bad, true, 2, 10), Verdict::Fail, "stable mismatch");
        assert_eq!(verdict(&bad, false, 10, 10), Verdict::Unstable);
        assert_eq!(
            verdict(&empty, true, 3, 10),
            Verdict::Retry,
            "empty is never a verdict"
        );
        assert_eq!(verdict(&empty, true, 10, 10), Verdict::Unstable);
    }

    #[test]
    fn detects_unrendered_capture() {
        let expected = [0u8, 0, 0, 255, 10, 20, 30, 255];
        let r = compare_rgba(&[0u8; 8], &expected, 2);
        assert!(r.unrendered);
        assert_eq!(r.to_string(), "capture is empty (renderer not ready yet)");
    }
}
