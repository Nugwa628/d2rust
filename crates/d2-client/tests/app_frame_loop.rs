//! The play mode's frame loop, headless: the app's own wiring
//! (`app::play::add_game`) over the app's single-player game
//! (`app::single_player`, synthetic tables, on its server thread) in a
//! Bevy `App` without a window, driven by `App::update`.
//!
//! - every update runs one bridge frame (pump: drain → tick → flush;
//!   receive), the server ticks on its host clock, the bridge receives
//!   the server's S→C message, and the world view builds a frame from the
//!   bridge's model through the client's view rules
//!   (`rules::OriginalView`, camera from the `ViewFeed`), once per server
//!   tick (no tick, no draw);
//! - the GPU compositor's render-graph node runs when an adapter exists:
//!   its output, read back from the presented texture, equals the CPU
//!   reference byte for byte. Without an adapter the GPU test reports the
//!   skip and passes (the CPU test still runs the whole loop).
//!
//! - the integration of the parallel client pieces (synthetic data): the
//!   world view reads its frames from the frame store (ids, CPU and GPU
//!   halves), UI text goes through `ui::text::layout_text`, and the audio
//!   frame plays cues from the sound pool, one decode per file; the
//!   level data selection falls back to the synthetic DRLG without game
//!   files (`app_single_player.rs`), and with them the same loop runs on
//!   the user's levels (ignored test).
//!
//! The host clock is a manual one shared with the test (the link owns
//! it; Bevy time never reaches the server). Rules, fonts, sounds and cues
//! here are test fixtures, not game rules.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::app::PluginGroup;
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::window::ExitCondition;
use d2_client::app::play::{add_game, send_create_game};
use d2_client::app::server_thread::ThreadLink;
use d2_client::app::single_player::{self, GameData, Link, Started, DEFAULT_SEED};
use d2_client::app::sound::{AudioParts, GameAudio, SoundTable};
use d2_client::assets::path::{CanonicalPath, MemorySource};
use d2_client::audio::{
    Cue, CueSource, Sound, SoundId, Trigger, TriggerQueue, TriggerSource, VoiceKind, VoiceParams,
    WavDecoder,
};
use d2_client::bridge::world::ClientWorld;
use d2_client::bridge::{BridgeResource, ClientUnit};
use d2_client::composite::{ComponentFrame, ComponentRequest, CompositeError, UnitParams};
use d2_client::frames::{FramePart, FrameSet, FrameSetKey, IndexFrame};
use d2_client::gpu_compositor::Gpu;
use d2_client::rules::{MapTile, OpenMode, Shake, TileList, UnitPosition, ViewSource};
use d2_client::scene::{BlendOp, DrawKey, ShadeChain};
use d2_client::ui::{
    GlyphLookup, GlyphPlacement, ImageRequest, NoPanelRules, Panel, PanelId, Point, TextError,
    TextOpts, TextRequest, TextRules, TextStyle, UiCtx, UiDraw, UiDrawSink, UiEvent, UiResponse,
    UiRoot, WidgetId,
};
use d2_client::world_view::node::NodeRuns;
use d2_client::world_view::present::{FrameStats, WorldViewTarget};
use d2_client::world_view::{
    build_frame, compose_cpu, text_sprites, RunningShake, TextFont, TextHooks, TileDraw, UiRules,
    UiSprite, UnitPose, Unspecified, ViewAssets, ViewError, ViewFeed, ViewRules, WorldViewState,
    WorldViewUi, VIEW,
};
use d2_formats::font::{FontTable, Glyph};
use d2_formats::palette::{Palette, Rgb};
use d2_server::seams::Clock;
use d2_sim::rng::Seed;

use crate::app_support;

/// The host clock, advanced by the test.
struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The app's game on clock `ms` (no client until its C→S 0x67).
fn game(ms: &Arc<AtomicU32>) -> (ThreadLink<Link<StepClock>>, Started) {
    single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap()
}

fn bridge(app: &App) -> &BridgeResource {
    app.world().resource::<BridgeResource>()
}

fn stats(app: &App) -> FrameStats {
    app.world()
        .resource::<WorldViewState>()
        .last
        .expect("a world view frame")
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn frame_loop_shakes_on_the_tick_time_base() {
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, _) = game(&ms);
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    add_game(&mut app, Box::new(link), true).unwrap();
    let mut seed = Seed::default();
    seed.set(0x1234_5678, 666);
    let shake = Shake::start(10, 100, 200, 100).unwrap();
    let feed = TestFeed {
        shake: Some(shake),
        seed,
        ..TestFeed::default()
    };
    app.insert_resource(WorldViewState::new(
        assets(),
        Box::new(TestRules),
        Box::new(feed),
    ));
    app.update();
    for _ in 0..3 {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
        app.update();
    }
    assert_eq!(bridge(&app).0.world().server_ticks, 3);

    // Ticks 1–3: t = 40, 80, 120 ms → a = 4, 8, 10; two draws each on the
    // player seed, once per drawn frame. The presented frame is tick 3's,
    // its origins moved by the third pair of offsets.
    let mut reference = seed;
    let mut offsets = (0, 0);
    for a in [4, 8, 10] {
        offsets = d2_client::rules::camera::shake_offsets(a, &mut reference);
    }
    let world = ClientWorld {
        server_ticks: 3,
        ..ClientWorld::default()
    };
    let mut at_tick_2 = TestFeed {
        shake: Some(shake),
        seed,
        ..TestFeed::default()
    };
    for a in [4, 8] {
        d2_client::rules::camera::shake_offsets(a, &mut at_tick_2.seed);
    }
    let cam = d2_client::world_view::frame_camera(&world, &mut at_tick_2)
        .unwrap()
        .unwrap();
    assert_eq!(at_tick_2.seed, reference);
    assert_eq!(
        (cam.tile.x, cam.tile.y),
        (600 + offsets.0, 1720 + offsets.1)
    );
    let a = assets();
    let mut fresh = TestFeed {
        shake: Some(shake),
        seed,
        ..TestFeed::default()
    };
    for a in [4, 8] {
        d2_client::rules::camera::shake_offsets(a, &mut fresh.seed);
    }
    let frame = build_frame(&world, &[], &TestRules, &mut fresh, &a).unwrap();
    let want = compose_cpu(&frame, &a).unwrap();
    let target = app.world().resource::<WorldViewTarget>();
    let image = app
        .world()
        .resource::<Assets<Image>>()
        .get(&target.image)
        .unwrap();
    assert_eq!(image.data.as_deref(), Some(&want[..]));
}

// ---- GPU node -------------------------------------------------------------------------

fn tile_key() -> FrameSetKey {
    FrameSetKey::new("data/global/tiles/app-test.dt1", FramePart::Tile(0)).unwrap()
}

/// A palette whose entries are distinct, so RGBA reveals the index.
fn palette() -> Palette {
    let mut p = Palette {
        colors: [Rgb::default(); 256],
    };
    for (i, c) in p.colors.iter_mut().enumerate() {
        let i = i as u8;
        *c = Rgb {
            r: i,
            g: 255 - i,
            b: i ^ 0x5a,
        };
    }
    p
}

/// One 37×23 frame of varying indices (0 = transparent).
fn assets() -> ViewAssets {
    let (w, h) = (37u32, 23u32);
    let pixels = (0..w * h).map(|i| (i % 251) as u8).collect();
    let mut a = ViewAssets::new(palette());
    a.frames
        .insert(
            tile_key(),
            FrameSet {
                frames: vec![IndexFrame::new(w, h, 0, 0, pixels).unwrap()],
            },
        )
        .unwrap();
    a
}

/// Moving-unit 16.16 coordinates whose client position is `(px, py)`
/// (a − b = 2 px, a + b = 4 py).
fn moving(px: i32, py: i32) -> UnitPosition {
    let (a, b) = (px + 2 * py, 2 * py - px);
    UnitPosition::Moving {
        x16: (a as u32) << 11,
        y16: (b as u32) << 11,
    }
}

/// Fixture camera feed (not a rule): the local player at client
/// (1000 + `step` × server ticks, 2000), open mode 0, the tile on floor
/// cells (26, 18), (27, 18) and (31, 13) (handed (40, 40), (120, 80),
/// (840, 40) at step 0: drawn at x − 80, the first partly off the left
/// edge), and an optional shake from tick 0 whose asks are logged.
#[derive(Default)]
struct TestFeed {
    step: i32,
    shake: Option<Shake>,
    seed: Seed,
    asked: Arc<Mutex<Vec<u64>>>,
}

impl ViewSource for TestFeed {
    fn unit_position(&self, _: &ClientUnit) -> Result<UnitPosition, String> {
        Err("no units in these scenes".into())
    }
    fn unit_offset(&self, _: &ClientUnit, _: &UnitPose) -> Result<(i32, i32), String> {
        Err("no units in these scenes".into())
    }
    fn map_tiles(&self, _: &ClientWorld, _: &ViewAssets) -> Result<Vec<MapTile>, ViewError> {
        let tile = |cell, minor| MapTile {
            cell,
            list: TileList::Floor,
            frame: ComponentFrame {
                set: tile_key(),
                index: 0,
            },
            blocks: Vec::new(),
            shade: ShadeChain::EMPTY,
            blend: BlendOp::Opaque,
            key: DrawKey::new(0, 0, minor, 0).unwrap(),
        };
        Ok(vec![
            tile((26, 18), 0),
            tile((27, 18), 1),
            tile((31, 13), 2),
        ])
    }
}

impl ViewFeed for TestFeed {
    fn player(&self, w: &ClientWorld) -> Result<Option<UnitPosition>, ViewError> {
        Ok(Some(moving(1000 + self.step * w.server_ticks as i32, 2000)))
    }
    fn open_mode(&self, _: &ClientWorld) -> Result<OpenMode, ViewError> {
        Ok(OpenMode::NONE)
    }
    fn shake(&self, w: &ClientWorld) -> Result<Option<RunningShake>, ViewError> {
        self.asked.lock().unwrap().push(w.server_ticks);
        Ok(self.shake.map(|shake| RunningShake {
            shake,
            start_tick: 0,
        }))
    }
    fn player_seed(&mut self, _: &ClientWorld) -> Result<&mut Seed, ViewError> {
        Ok(&mut self.seed)
    }
    fn blank_screen(&self, _: &ClientWorld) -> Result<bool, ViewError> {
        Ok(true)
    }
}

/// Fixture rules: no tile of their own (the game's placement answers
/// tiles, from [`TestFeed`]), no unit drawn.
struct TestRules;

impl ViewRules for TestRules {
    fn tiles(&self, _: &ClientWorld, _: &ViewAssets) -> Result<Vec<TileDraw>, ViewError> {
        unreachable!("OriginalView answers tiles")
    }
    fn unit_pose(&self, w: &ClientWorld, u: &ClientUnit) -> Result<Option<UnitPose>, ViewError> {
        Unspecified.unit_pose(w, u)
    }
    fn unit_params(
        &self,
        w: &ClientWorld,
        u: &ClientUnit,
        p: &UnitPose,
    ) -> Result<UnitParams, ViewError> {
        Unspecified.unit_params(w, u, p)
    }
    fn component_frame(
        &self,
        u: &ClientUnit,
        p: &UnitPose,
        r: &ComponentRequest<'_>,
    ) -> Result<ComponentFrame, CompositeError> {
        Unspecified.component_frame(u, p, r)
    }
    fn place(
        &self,
        u: &ClientUnit,
        p: &UnitPose,
        r: &ComponentRequest<'_>,
        i: &IndexFrame,
    ) -> Result<(i32, i32), CompositeError> {
        Unspecified.place(u, p, r, i)
    }
    fn shade(
        &self,
        u: &ClientUnit,
        r: &ComponentRequest<'_>,
    ) -> Result<ShadeChain, CompositeError> {
        Unspecified.shade(u, r)
    }
    fn blend(&self, u: &ClientUnit, r: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
        Unspecified.blend(u, r)
    }
}

impl UiRules for TestRules {
    fn ui_image(&self, r: &ImageRequest, a: &ViewAssets) -> Result<UiSprite, ViewError> {
        Unspecified.ui_image(r, a)
    }
    fn ui_text(&self, r: &TextRequest, a: &ViewAssets) -> Result<Vec<UiSprite>, ViewError> {
        Unspecified.ui_text(r, a)
    }
    fn ui_pass(&self) -> Result<u32, ViewError> {
        Unspecified.ui_pass()
    }
}

/// Removes the row padding of a texture readback.
fn unpad(data: &[u8], width: u32, height: u32) -> Vec<u8> {
    let row = (width * 4) as usize;
    let stride = data.len() / height as usize;
    assert!(stride >= row && data.len() == stride * height as usize);
    data.chunks_exact(stride)
        .flat_map(|r| &r[..row])
        .copied()
        .collect()
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn gpu_node_composes_the_frame_into_the_presented_texture() {
    let adapter = match Gpu::headless() {
        Ok((_, info)) => info,
        Err(e) => {
            eprintln!("skipped: no GPU adapter ({e}); the CPU frame loop test covers the rest");
            return;
        }
    };
    eprintln!(
        "adapter: {} ({:?}, {})",
        adapter.name, adapter.backend, adapter.driver
    );
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, _) = game(&ms);
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>()
            .disable::<bevy::audio::AudioPlugin>()
            .disable::<bevy::log::LogPlugin>(),
    );
    add_game(&mut app, Box::new(link), true).unwrap();
    app.insert_resource(WorldViewState::new(
        assets(),
        Box::new(TestRules),
        Box::new(TestFeed::default()),
    ));
    app.finish();
    app.cleanup();

    let captured: Arc<Mutex<Option<Vec<u8>>>> = Arc::default();
    let mut armed = false;
    for _ in 0..60 {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
        let runs = app.world().resource::<NodeRuns>().get();
        if runs > 0 && !armed {
            // Read the presented texture back once the node has written it.
            let image = app.world().resource::<WorldViewTarget>().image.clone();
            let sink = captured.clone();
            app.world_mut().spawn(Readback::texture(image)).observe(
                move |ev: On<ReadbackComplete>, mut commands: Commands| {
                    let mut s = sink.lock().unwrap();
                    if s.is_none() {
                        *s = Some(ev.data.clone());
                        commands.entity(ev.entity).despawn();
                    }
                },
            );
            armed = true;
        }
        if captured.lock().unwrap().is_some() {
            break;
        }
    }
    let s = stats(&app);
    assert!(s.gpu, "the GPU path was chosen");
    assert_eq!(s.items, 3);
    assert!(app.world().resource::<NodeRuns>().get() > 0, "the node ran");
    let data = captured
        .lock()
        .unwrap()
        .take()
        .expect("the presented texture was read back");

    let world = ClientWorld::default();
    let a = assets();
    let frame = build_frame(&world, &[], &TestRules, &mut TestFeed::default(), &a).unwrap();
    let want = compose_cpu(&frame, &a).unwrap();
    let got = unpad(&data, VIEW.width, VIEW.height);
    let differ = got
        .as_chunks::<4>()
        .0
        .iter()
        .zip(want.as_chunks::<4>().0)
        .filter(|(g, w)| g != w)
        .count();
    assert_eq!(differ, 0, "pixels differing from the CPU reference");

    // Each next frame waits for the previous frame's indices (the frame
    // cycle's base): the node keeps composing only if
    // the readback reaches the cycle.
    for _ in 0..120 {
        if app.world().resource::<NodeRuns>().get() >= 4 {
            break;
        }
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
    }
    assert!(
        app.world().resource::<NodeRuns>().get() >= 4,
        "the frame cycle advanced through the index readback"
    );
    let cycle = app.world().resource::<WorldViewState>().cycle.clone();
    assert_eq!(
        cycle.pixels(),
        &d2_client::scene::compose(&frame.items, &a.frames, &a.maps, VIEW).unwrap()[..],
        "a static frame over its own previous frame"
    );
}

// ---- integration: frame store, text layout, sound pool (synthetic) ---------------------

fn font_path() -> CanonicalPath {
    CanonicalPath::new("data/local/font/app-test.tbl").unwrap()
}

fn glyph_key() -> FrameSetKey {
    FrameSetKey::new("data/local/font/app-test.dc6", FramePart::Dir(0)).unwrap()
}

fn glyph(code: u8, frame: u8) -> Glyph {
    Glyph {
        code: u16::from(code),
        unknown1: 0,
        width: 6,
        height: 8,
        unknown2: 0,
        unknown3: 0,
        frame: u16::from(frame),
        unknown5: 0,
    }
}

/// [`assets`] plus a 256-record font (records by position):
/// its two-frame glyph DC6 is inserted after the tile, so the glyph
/// frames are store ids 1 and 2. `H` is record 72 and frame 1, `i`
/// record 105 and frame 0 (record ≠ frame).
fn text_assets() -> ViewAssets {
    let mut a = assets();
    let frame = |v: u8| IndexFrame::new(6, 8, 0, 0, vec![v; 48]).unwrap();
    a.frames
        .insert(
            glyph_key(),
            FrameSet {
                frames: vec![frame(201), frame(202)],
            },
        )
        .unwrap();
    a.fonts.insert(
        font_path(),
        FontTable {
            version: 1,
            unknown: 0,
            count: 256,
            height: 8,
            width: 6,
            glyphs: (0..=255u8).map(|c| glyph(c, u8::from(c == b'H'))).collect(),
        },
    );
    a
}

/// Fixture layout: every code unit drawn, 7 pixels apart, color 0.
struct FixedAdvance;

impl TextRules for FixedAdvance {
    fn place(
        &self,
        _: &GlyphLookup<'_>,
        text: &[u16],
        origin: Point,
        _: TextStyle,
        _: &TextOpts,
    ) -> Result<Vec<GlyphPlacement>, TextError> {
        Ok(text
            .iter()
            .enumerate()
            .map(|(i, &code)| GlyphPlacement {
                code,
                at: Point::new(origin.x + 7 * i as i32, origin.y),
                color: 0,
            })
            .collect())
    }
}

/// Fixture rules: the GPU test's tiles, UI text in the test font (pass 5,
/// opaque, unshaded), no UI image.
struct TextTestRules;

impl ViewRules for TextTestRules {
    fn tiles(&self, w: &ClientWorld, a: &ViewAssets) -> Result<Vec<TileDraw>, ViewError> {
        TestRules.tiles(w, a)
    }
    fn unit_pose(&self, w: &ClientWorld, u: &ClientUnit) -> Result<Option<UnitPose>, ViewError> {
        TestRules.unit_pose(w, u)
    }
    fn unit_params(
        &self,
        w: &ClientWorld,
        u: &ClientUnit,
        p: &UnitPose,
    ) -> Result<UnitParams, ViewError> {
        TestRules.unit_params(w, u, p)
    }
    fn component_frame(
        &self,
        u: &ClientUnit,
        p: &UnitPose,
        r: &ComponentRequest<'_>,
    ) -> Result<ComponentFrame, CompositeError> {
        TestRules.component_frame(u, p, r)
    }
    fn place(
        &self,
        u: &ClientUnit,
        p: &UnitPose,
        r: &ComponentRequest<'_>,
        i: &IndexFrame,
    ) -> Result<(i32, i32), CompositeError> {
        TestRules.place(u, p, r, i)
    }
    fn shade(
        &self,
        u: &ClientUnit,
        r: &ComponentRequest<'_>,
    ) -> Result<ShadeChain, CompositeError> {
        TestRules.shade(u, r)
    }
    fn blend(&self, u: &ClientUnit, r: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
        TestRules.blend(u, r)
    }
}

impl TextHooks for TextTestRules {
    fn text_font(&self, _: TextStyle) -> Result<TextFont, ViewError> {
        Ok(TextFont {
            table: font_path(),
            glyphs: glyph_key(),
        })
    }
    fn text_rules(&self) -> &dyn TextRules {
        &FixedAdvance
    }
    fn glyph_look(&self, _: i32, _: u8) -> Result<(ShadeChain, BlendOp), ViewError> {
        Ok((ShadeChain::EMPTY, BlendOp::Opaque))
    }
}

impl UiRules for TextTestRules {
    fn ui_image(&self, r: &ImageRequest, a: &ViewAssets) -> Result<UiSprite, ViewError> {
        Unspecified.ui_image(r, a)
    }
    fn ui_text(&self, r: &TextRequest, a: &ViewAssets) -> Result<Vec<UiSprite>, ViewError> {
        text_sprites(self, r, a)
    }
    fn ui_pass(&self) -> Result<u32, ViewError> {
        Ok(5)
    }
}

fn hi() -> UiDraw {
    UiDraw::Text(TextRequest {
        text: "Hii".encode_utf16().collect(),
        at: Point::new(300, 200),
        style: TextStyle::default(),
        opts: TextOpts::default(),
        clip: d2_client::ui::Rect::new(0, 0, 800, 600),
    })
}

/// A panel that draws "Hii" at (300, 200).
struct TextPanel;

impl Panel for TextPanel {
    fn id(&self) -> PanelId {
        PanelId(1)
    }
    fn rect(&self) -> d2_client::ui::Rect {
        d2_client::ui::Rect::new(290, 190, 100, 30)
    }
    fn draw(&self, _: &UiCtx, out: &mut dyn UiDrawSink) {
        out.push(hi());
    }
    fn hit(&self, _: Point) -> Option<WidgetId> {
        None
    }
    fn event(&mut self, _: UiEvent, _: &UiCtx) -> UiResponse {
        UiResponse::Ignored
    }
}

const SFX: &str = "data/global/sfx/app-test.wav";

/// Fixture table: sound 1 is [`SFX`].
struct OneSound;

impl SoundTable for OneSound {
    fn file(&self, id: SoundId) -> Option<Arc<str>> {
        (id == SoundId(1)).then(|| Arc::from(SFX))
    }
}

/// Fixture decoder: the file's bytes as mono 8-bit-to-16 samples (not a
/// WAV parse), so the voice log shows the pool's bytes reached it.
struct BytesAsSamples;

impl WavDecoder for BytesAsSamples {
    fn decode(&self, _: &CanonicalPath, bytes: &[u8]) -> Result<Sound, String> {
        Sound::new(22050, 1, bytes.iter().map(|&b| i16::from(b) << 8).collect())
            .map_err(|e| e.to_string())
    }
}

/// Fixture cues: on the first drain, sound 1 twice at tick 1.
struct TwoStarts(bool);

impl CueSource for TwoStarts {
    fn drain_cues(&mut self, queue: &mut TriggerQueue) {
        if std::mem::replace(&mut self.0, true) {
            return;
        }
        for cause in ["first", "second"] {
            queue.push(Cue::Start(Trigger {
                tick: 1,
                source: TriggerSource::Sim,
                sound: SoundId(1),
                params: VoiceParams {
                    vol: 0,
                    pan: 0,
                    looped: false,
                    priority: 0,
                    group: 0,
                },
                cause: cause.into(),
            }));
        }
    }
}

fn audio() -> GameAudio {
    let mut source = MemorySource::default();
    source.insert(r"data\global\sfx\app-test.wav", vec![3; 8192]);
    let mut parts = AudioParts::unspecified(Arc::new(source));
    parts.decoder = Box::new(BytesAsSamples);
    parts.table = Box::new(OneSound);
    parts.cues = Box::new(TwoStarts(false));
    GameAudio::new(parts)
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn frame_loop_uses_the_frame_store_text_layout_and_sound_pool() {
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, _) = game(&ms);
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    add_game(&mut app, Box::new(link), true).unwrap();
    app.insert_resource(WorldViewState::new(
        text_assets(),
        Box::new(TextTestRules),
        Box::new(TestFeed::default()),
    ))
    .insert_resource(audio());
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    root.add(Box::new(TextPanel)).unwrap();
    root.open(PanelId(1)).unwrap();
    app.insert_non_send(WorldViewUi::new(root, Box::new(d2_client::ui::NoStrings)));

    // Frame 1: no tick yet; the cues are queued, tick 0 presented.
    app.update();
    let audio_stats = app.world().resource::<GameAudio>().stats.clone();
    assert_eq!((audio_stats.frame, audio_stats.presented), (1, 0));
    // Frames 2–3: a tick each; tick 1 presented released both starts.
    for _ in 0..2 {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
    }
    assert_eq!(bridge(&app).0.world().server_ticks, 2);
    assert_eq!(app.world().resource::<GameAudio>().stats.presented, 2);

    // (1) + (2) text: three tiles, three glyphs. The glyph items carry the
    // store ids of their DC6 frames (H → frame 1 → id 2, i → frame 0 → id
    // 1), at the rules' placements, after the tiles (pass 5).
    let s = stats(&app);
    assert_eq!((s.items, s.gpu), (6, false));
    let a = text_assets();
    let world = ClientWorld {
        server_ticks: 2,
        ..ClientWorld::default()
    };
    let frame = build_frame(
        &world,
        &[hi()],
        &TextTestRules,
        &mut TestFeed::default(),
        &a,
    )
    .unwrap();
    let glyphs: Vec<_> = frame.items[3..]
        .iter()
        .map(|i| (i.frame.0, i.x, i.y))
        .collect();
    assert_eq!(glyphs, vec![(2, 300, 200), (1, 307, 200), (1, 314, 200)]);
    let want = compose_cpu(&frame, &a).unwrap();
    let target = app.world().resource::<WorldViewTarget>();
    let image = app
        .world()
        .resource::<Assets<Image>>()
        .get(&target.image)
        .unwrap();
    assert_eq!(image.data.as_deref(), Some(&want[..]));
    // The glyph pixels are in the presented frame (index 202 at "H").
    let at = |x: u32, y: u32| {
        let o = ((y * VIEW.width + x) * 4) as usize;
        want[o..o + 4].to_vec()
    };
    let c = palette().colors[202];
    assert_eq!(at(300, 200), vec![c.r, c.g, c.b, 255]);

    // (2) sound: the output edge mixes (here the test, no device): both
    // starts play the pool's one decode of the file.
    let audio = app.world().resource::<GameAudio>();
    let mut engine = audio.engine.lock().unwrap();
    engine.mix_block();
    assert_eq!(engine.voices().len(), 2);
    let starts: Vec<_> = engine
        .log()
        .events
        .iter()
        .map(|e| (e.tick, e.kind, e.file.as_str(), e.error, e.cause.as_str()))
        .collect();
    assert_eq!(
        starts,
        vec![
            (1, VoiceKind::Start, SFX, false, "first"),
            (1, VoiceKind::Start, SFX, false, "second"),
        ]
    );
    drop(engine);
    assert_eq!(audio.pool.lock().unwrap().decodes(), 1);

    // The next frame reports the decode and no error.
    ms.fetch_add(40, Ordering::SeqCst);
    app.update();
    let audio_stats = &app.world().resource::<GameAudio>().stats;
    assert_eq!(
        (
            audio_stats.decodes,
            audio_stats.load_errors,
            audio_stats.engine_errors
        ),
        (1, 0, 0)
    );
}

// The default hooks refuse: with the app's own hooks, text needs the
// game font (font 0, not loaded here) and a sound start fails (no
// table file).
#[test]
fn placeholder_hooks_refuse_text_and_sounds() {
    let a = text_assets();
    let UiDraw::Text(req) = hi() else {
        unreachable!()
    };
    let e = Unspecified.ui_text(&req, &a).unwrap_err();
    assert!(
        matches!(&e, ViewError::FontMissing(p) if p.as_str() == "data/local/font/latin/font8.tbl"),
        "{e}"
    );

    let mut parts = AudioParts::empty();
    parts.cues = Box::new(TwoStarts(false));
    let audio = GameAudio::new(parts);
    let mut engine = audio.engine.lock().unwrap();
    let mut queue = TriggerQueue::new();
    TwoStarts(false).drain_cues(&mut queue);
    for (_, cue) in queue.take_due(1) {
        engine.queue_mut().push(cue);
    }
    engine.present(1).unwrap();
    engine.mix_block();
    assert!(engine.voices().is_empty());
    assert!(engine.log().events.iter().all(|e| e.error));
    assert_eq!(audio.pool.lock().unwrap().decodes(), 0);
}

/// The same loop on the user's levels (drlg-data providers): the game
/// builds from `D2_GAME_DIR`, the server ticks 100 times without error.
#[test]
#[ignore = "needs the game files in D2_GAME_DIR"]
fn frame_loop_runs_on_the_users_levels() {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
    let data = GameData::select(Some(dir.as_ref())).unwrap();
    assert!(matches!(data, GameData::Live(_)));
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, started) = single_player::start(data, DEFAULT_SEED, StepClock(ms.clone())).unwrap();
    eprintln!("started: {started:?}");
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    add_game(&mut app, Box::new(link), true).unwrap();
    // The session sequence: 0x67 now, 0x6B after the flush with 0x02.
    send_create_game(&mut app).unwrap();
    app.update();
    for _ in 0..100 {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
    }
    let w = bridge(&app).0.world();
    assert_eq!((w.frames, w.server_ticks), (101, 100));
    assert!(w.in_game && w.local_player.is_some());
}
