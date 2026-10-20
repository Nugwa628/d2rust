//! Bevy edge of the world view: after the bridge frame (`PreUpdate`), one
//! `Update` system runs the UI, builds the game frame
//! ([`super::build_frame`]: the frame's camera from the [`super::ViewFeed`],
//! then the view rules), composes it and shows the game frame's image with a
//! sprite, scaled for the window by `present_scale`.
//!
//! Time base: one game frame per drawn server tick, built from the model as
//! it stands after that tick. A Bevy frame whose bridge frame ran no tick
//! builds nothing and keeps the presented image; between ticks
//! [`super::smooth`] shows the frame's world layer shifted along the last
//! camera step, so the view scrolls at display rate while the tick frames
//! themselves stay exactly as composed.
//!
//! Compose is the GPU compute compositor when [`WorldViewGpu`] exists (the
//! frame is packed here and composed by the render-graph node,
//! [`super::node`], straight into the image's texture), else the CPU
//! reference written into the image.
//!
//! The game frame persists: [`WorldViewState::cycle`] holds the index
//! framebuffer between frames, and each frame's clears come from
//! `cycle.plan(blank_screen)` with the feed's BlankScreen. CPU:
//! `cycle.compose`. GPU: the frame is packed onto `cycle.pixels()`, the
//! node composes it and reads the indices back ([`NodeIndices`]), and the
//! next frame is built over them once they are committed to the cycle. In
//! live play they come back a frame late; once the node has composed the
//! job ([`NodeComposed`]) the next frame is built anyway, composed over the
//! GPU's own copy of those indices, and the cycle only counts the frame
//! ([`crate::scene::FrameCycle::pass`]) until a later read-back catches it
//! up. A check run or a dump builds the next frame only after the commit
//! (a Bevy frame that would build before that waits, like one whose bridge
//! frame ran no tick).
//!
//! The bridge frame's UI and sound outputs are applied by
//! [`deliver_outputs`] right after the bridge frame, before the input and
//! UI systems: UI outputs by the game's UI, sound outputs (and the sounds
//! the UI makes for them) appended to [`UiSounds`] in list order.
//!
//! Inert until the app inserts [`crate::bridge::BridgeResource`] and
//! [`WorldViewState`]: nothing here builds a server or chooses rules.
//! Errors go to Bevy's error handler; a failed GPU frame never falls back
//! to the CPU one.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::message::{MessageCursor, Messages};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::Msaa;
use bevy::window::PrimaryWindow;
use std::sync::Arc;

use crate::audio::driver::SoundRequest;
use crate::bridge::link::ServerLink;
use crate::bridge::mirror::{bridge_frame, mirror_units};
use crate::bridge::output::{dispatch, Output};
use crate::bridge::world::UnitKey;
use crate::bridge::{Bridge, BridgeError, BridgeResource, FrameOutputs};
use crate::controls::click::ClickOut;
use crate::controls::Bindings;
use crate::frames::atlas::AtlasPage;
use crate::ui::original::OriginalUi;
use crate::ui::{edge, FramePos, PointerButton, StringLookup, UiEvent, UiRoot};

use super::feed::{build_frame_placed, ViewFeed};
use super::node::{add_node, ComposeJob, NodeComposed, NodeIndices};
use super::panel_art::PanelArtLoader;
use super::sharp::SharpMaterial;
use super::ui_bind::{run_ui_with, world_clicks, TextAssetLoader, UiQueue, UiRules};
use super::walk::PreviewWalk;
use super::{compose_cycle_cpu, play_view, GpuAtlas, ViewAssets, ViewRules};
use crate::scene::{FrameCycle, FramePlan};

/// Render layer of the presented frame and its camera, so the world view
/// never mixes with other sprites of the app.
pub const PRESENT_LAYER: usize = 31;

/// Atlas pages the in-app GPU path may use (64 × 4 MiB).
pub const GPU_ATLAS_PAGES: u32 = 64;

/// Ticks in a row live play waits for the render world to compose a job
/// before it gives the job up and draws on (about a second at 60 ticks a
/// second; a composed job is back within a tick or two).
pub const NODE_STALL_TICKS: u32 = 60;

/// Rules of both halves, as one object.
pub trait WorldRules: ViewRules + UiRules {}
impl<T: ViewRules + UiRules + ?Sized> WorldRules for T {}

/// The world view's inputs besides the bridge: the assets, the game-rule
/// hooks (`rules`: pose, component frames, draw keys, shading, blend, UI)
/// and the camera feed (`feed`: player, open mode, shake, unit positions,
/// map tiles). Placement is the game's own (`rules::OriginalView`), not a
/// hook.
#[derive(Resource)]
pub struct WorldViewState {
    pub assets: ViewAssets,
    pub rules: Box<dyn WorldRules + Send + Sync>,
    pub feed: Box<dyn ViewFeed + Send + Sync>,
    /// The persistent index framebuffer, [`play_view`]
    /// sized: the last presented frame once committed.
    pub cycle: FrameCycle,
    /// Counts of the last frame, for logs and tests.
    pub last: Option<FrameStats>,
    /// The drawn animation clock of the units in view (`anim_clock`).
    pub drawn_anim: super::anim_clock::DrawnAnim,
    /// Frames skipped because they could not be built
    /// ([`skip_failed_frame`]), for logs and tests.
    pub skipped_frames: u64,
    /// The world-click state (held buttons, repeat, latch).
    pub click: crate::controls::click::ClickState,
    /// The game's automap, when the app supplied its
    /// tables; `None`: no automap.
    pub automap: Option<crate::ui::automap::session::AutomapSession>,
    /// The automap's draw sink (`super::automap_view`); `None`: not drawn.
    pub automap_view: Option<super::automap_view::AutomapView>,
    /// The level backgrounds of pass 1 (`super::background_view`);
    /// `None`: not drawn.
    pub background_view: Option<super::background_view::BackgroundView>,
    /// The play preview (decision D1, [`super::preview`]): a frame whose
    /// build fails is logged (each message once) and not presented,
    /// instead of failing the app. `false`: strict (a frame error fails
    /// the app).
    pub preview: bool,
    /// The last logged preview frame error.
    preview_error: Option<String>,
    /// The last drawn frame's item tags (tiles, units, UI) and UI draws,
    /// in draw order: what the frame shows, for logs and tests.
    pub last_tags: Vec<crate::scene::ItemTag>,
    pub last_ui: Vec<crate::ui::UiDraw>,
    /// The hover target the previous pass left (`ClickView::prev_hover`).
    pub prev_hover: Option<crate::bridge::world::UnitKey>,
    /// The preview's pending interaction (`super::interact`).
    pub interact: super::interact::PreviewInteract,
    /// Ground items (`super::ground_items`): the app hands in the item art
    /// rows and the archives; the default draws nothing.
    pub ground_items: super::ground_items::GroundItems,
    /// The click on a corpse (`super::corpse_click`).
    pub corpse_clicks: super::corpse_click::CorpseClicks,
    /// Client missiles and cast / state overlays (`super::missiles`).
    pub missiles: super::missiles::Missiles,
    /// The object mouse-over label (`super::object_label`).
    pub object_labels: super::object_label::ObjectLabels,
    pub monster_labels: super::monster_label::MonsterLabels,
    /// The pick of the last drawn tick ([`tick_pick`]): what the
    /// mouse-over labels and the highlight follow.
    pub rect_hover: Option<UnitKey>,
    /// The server tick of the last [`tick_pick`]: one pick per client
    /// update, drawn or not.
    pub pick_tick: u64,
    /// The last drawn frame's camera (`super::visibility`).
    pub camera: super::visibility::SharedCamera,
    /// The last drawn tick's local-player position and shake: decided
    /// once per drawn tick,
    /// read by the UI, the pick and the labels on every loop pass until
    /// the next tick is drawn.
    pub anchor: Option<crate::rules::camera::FrameAnchor>,
    /// The server tick whose anchor and world-draw seed rolls a pass whose
    /// draw waited for the GPU took ahead: the pass that draws it takes
    /// neither again, so each tick rolls its draw seed once.
    rolled_tick: u64,
    /// The model's act loads already handed to the cycle
    /// ([`note_act_loads`]).
    act_loads: u64,
}

/// An act load (each S→C 0x03 the model handled since the last call) sets
/// the post-draw clear counter to 1, so the next presented frame is all
/// index 0: the old act's picture never shows under the new one.
pub fn note_act_loads(cycle: &mut FrameCycle, seen: &mut u64, loads: u64) {
    if loads != *seen {
        *seen = loads;
        cycle.set_post_clear(1);
    }
}

/// The hover target of this tick, picked once per client update, after
/// the update and before the draw. The units' rectangles and layer boxes
/// are their cels of this tick (the update advanced the frames), placed
/// with `camera`, the last drawn frame's (the draw sets the new one); the
/// ground items and the warp tiles under the same camera. The ranking and
/// the sticky rule are `pick_layers`'.
fn tick_pick(
    world: &crate::bridge::world::ClientWorld,
    state: &WorldViewState,
    camera: crate::rules::camera::Camera,
    mouse: (i32, i32),
    player: Option<(i32, i32)>,
) -> Option<UnitKey> {
    let feed = state.feed.as_ref();
    let view = crate::rules::OriginalView::new(camera, state.rules.as_ref(), feed)
        .with_edge_clip(feed.edge_clip());
    let (mut rects, layers, _) = super::unit_body_rects(world, &view, &state.assets);
    for d in state
        .ground_items
        .draws(world, &camera, &state.assets, None)
    {
        let Some(f) = state.assets.frames.frame(d.item.frame) else {
            continue;
        };
        let (l, t) = (d.item.x, d.item.y);
        rects.insert(
            UnitKey::new(crate::bridge::items::ITEM, d.guid),
            (l, t, l + f.width as i32, t + f.height as i32),
        );
    }
    rects.extend(crate::bridge::hover::tile_rects(world, &camera));
    let point = |k: UnitKey| {
        let u = world.units.get(&k)?;
        let c = feed.unit_position(u).ok()?.client();
        Some((c.x, c.y))
    };
    crate::bridge::hover::pick_layers(
        world,
        &rects,
        &layers,
        mouse,
        state.rect_hover,
        player,
        &point,
    )
}

/// The pointer of a pass over the frame: the script's or the host's
/// cursor (`mouse`), else the UI's own pointer (a check run with no host
/// pointer: the UI pointer starts at (320, 240)); none without either.
fn hover_pointer(ui: &WorldViewUi, mouse: (i32, i32)) -> Option<(i32, i32)> {
    match (ui.cursor, ui.original.as_ref()) {
        (Some(FramePos::Inside(_)), _) => Some(mouse),
        (None, Some(o)) => {
            let p = o.pointer();
            Some((p.x, p.y))
        }
        _ => None,
    }
}

/// The point the pick tests: none below H − 49 (the control panel at the
/// bottom of the frame hides the world there), and none over an open game
/// panel ([`crate::ui::original::covers_world`]: the inventory, character,
/// stash, cube, shop, NPC windows, the control panel with a popped belt).
/// Game panels cover the world, so the world under them is not hovered at
/// all: no unit highlight, no monster or object name label and no hovered
/// ground-item label.
pub(crate) fn pick_point(pointer: Option<(i32, i32)>, root: &UiRoot) -> Option<(i32, i32)> {
    let h = crate::rules::camera::FrameSize::play().height;
    pointer.filter(|&(_, y)| y <= h - 49).filter(|&(x, y)| {
        !root.open_panel_at(
            crate::ui::Point::new(x, y),
            crate::ui::original::covers_world,
        )
    })
}

impl WorldViewState {
    pub fn new(
        assets: ViewAssets,
        rules: Box<dyn WorldRules + Send + Sync>,
        feed: Box<dyn ViewFeed + Send + Sync>,
    ) -> Self {
        WorldViewState {
            assets,
            rules,
            feed,
            cycle: FrameCycle::new(play_view().width, play_view().height)
                .expect("the play frame is taller than the uncleared band"),
            last: None,
            drawn_anim: Default::default(),
            skipped_frames: 0,
            click: Default::default(),
            automap: None,
            automap_view: None,
            background_view: None,
            preview: false,
            preview_error: None,
            last_tags: Vec::new(),
            last_ui: Vec::new(),
            prev_hover: None,
            interact: Default::default(),
            ground_items: Default::default(),
            corpse_clicks: Default::default(),
            missiles: Default::default(),
            object_labels: Default::default(),
            monster_labels: Default::default(),
            rect_hover: None,
            pick_tick: 0,
            camera: Default::default(),
            anchor: None,
            rolled_tick: 0,
            act_loads: 0,
        }
    }
}

/// A check run's recorded frame schedule (`play --frame-schedule`): the
/// client updates on which the recorded run drew a frame, each with the
/// host clock (milliseconds) of the frame's cursor step, and the cursor
/// timers at the first frame. With a
/// schedule the world view draws exactly those ticks (one frame each) and
/// nothing between them; the host clock and the frame schedule are
/// recorded inputs, not game behaviour.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameSchedule {
    /// Drawn tick → the clock of its cursor step (`None`: not recorded,
    /// allowed for the last frame only: its step follows the dump).
    pub frames: std::collections::BTreeMap<u64, Option<u32>>,
    /// Drawn tick → the recorded light quality (a host input: it follows
    /// the measured draw rate),
    /// where the schedule has the column.
    pub quality: std::collections::BTreeMap<u64, u8>,
    /// Drawn tick → the pointer the frame started with (frame pixels): where
    /// the OS put it after a panel's cursor jump.
    pub pointer: std::collections::BTreeMap<u64, (i32, i32)>,
    /// Drawn tick → the clock of every frame the recorded run drew on it,
    /// in order (more than one when a loop pass drew without a client
    /// update, after that tick's sound tick): each draw steps the cursor.
    pub draws: std::collections::BTreeMap<u64, Vec<Option<u32>>>,
    /// Drawn tick → the cursor's recorded `idle_since` at the frame's
    /// start (where the schedule has the column): a later value than the
    /// cursor's own is a host window message.
    pub idle: std::collections::BTreeMap<u64, u32>,
    /// Drawn tick → the monster graphics-ready flags at the frame's start
    /// (the `ready` column; a host input: graphics load in the
    /// background), where recorded.
    pub ready: std::collections::BTreeMap<u64, crate::bridge::world::GfxReady>,
    /// `last_step` and `idle_since` of the cursor at the first frame.
    pub cursor_last: u32,
    pub cursor_idle: u32,
    /// The schedule was applied (cursor timers, weather replay off).
    pub started: bool,
}

impl FrameSchedule {
    /// Parses the `frame-schedule 1` text: `# frame-schedule 1`, `#
    /// tick_rate 60`, `# cursor_last N`, `# cursor_idle N`, a `tick\tnow` header, then one
    /// row per drawn frame (`now` `-` when not recorded).
    pub fn parse(text: &str) -> Result<Self, String> {
        crate::recording::validate(
            crate::recording::text_rate(text)?,
            d2_sim::tick::TickRate::GAME,
        )?;
        let mut s = FrameSchedule::default();
        let (mut version, mut last, mut idle) = (false, None, None);
        for (n, line) in text.lines().enumerate() {
            let at = |m: &str| format!("frame schedule line {}: {m}", n + 1);
            if let Some(h) = line.strip_prefix('#') {
                let mut w = h.split_whitespace();
                match (w.next(), w.next()) {
                    (Some("frame-schedule"), Some("1")) => version = true,
                    (Some("cursor_last"), Some(v)) => {
                        last = Some(v.parse().map_err(|_| at("cursor_last"))?)
                    }
                    (Some("cursor_idle"), Some(v)) => {
                        idle = Some(v.parse().map_err(|_| at("cursor_idle"))?)
                    }
                    _ => {}
                }
                continue;
            }
            if line.trim().is_empty() || line.starts_with("tick") {
                continue;
            }
            let mut c = line.split('\t');
            let tick: u64 = c
                .next()
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| at("tick"))?;
            let now = match c.next() {
                Some("-") => None,
                Some(v) => Some(v.parse().map_err(|_| at("now"))?),
                None => return Err(at("no now column")),
            };
            s.frames.insert(tick, now);
            s.draws.entry(tick).or_default().push(now);
            if let Some(q) = c.next().filter(|q| *q != "-") {
                s.quality
                    .insert(tick, q.parse().map_err(|_| at("quality"))?);
            }
            if let (Some(x), Some(y)) = (c.next(), c.next()) {
                if x != "-" && y != "-" {
                    let x = x.parse().map_err(|_| at("cursor_x"))?;
                    let y = y.parse().map_err(|_| at("cursor_y"))?;
                    s.pointer.insert(tick, (x, y));
                }
            }
            if let Some(v) = c.next().filter(|v| *v != "-") {
                s.idle
                    .insert(tick, v.parse().map_err(|_| at("cursor_idle"))?);
            }
            if let Some(v) = c.next().filter(|v| *v != "-") {
                s.ready
                    .insert(tick, parse_ready(v).ok_or_else(|| at("ready"))?);
            }
        }
        if !version {
            return Err("frame schedule: no `# frame-schedule 1` line".into());
        }
        s.cursor_last = last.ok_or("frame schedule: no cursor_last")?;
        s.cursor_idle = idle.ok_or("frame schedule: no cursor_idle")?;
        let last_tick = s.frames.keys().next_back().copied();
        if let Some(t) = s
            .frames
            .iter()
            .find(|(t, now)| now.is_none() && Some(**t) != last_tick)
            .map(|(t, _)| *t)
        {
            return Err(format!(
                "frame schedule: drawn tick {t} has no recorded host clock (only the last frame may lack it)"
            ));
        }
        if s.frames.is_empty() {
            return Err("frame schedule: no drawn frame".into());
        }
        Ok(s)
    }

    /// The clock of the first frame drawn on `tick` (the pass with its
    /// client update); `now` when the tick was drawn once.
    pub fn first_now(&self, tick: u64) -> Option<u32> {
        match self.draws.get(&tick) {
            Some(d) if d.len() > 1 => d[0],
            _ => self.now(tick),
        }
    }

    /// The clocks of the frames drawn on `tick` after its first one.
    pub fn extra_draws(&self, tick: u64) -> &[Option<u32>] {
        self.draws
            .get(&tick)
            .map_or(&[][..], |d| d.get(1..).unwrap_or(&[]))
    }

    /// Whether the recorded run drew a frame on `tick`.
    pub fn drawn(&self, tick: u64) -> bool {
        self.frames.contains_key(&tick)
    }

    /// The host clock of `tick`'s cursor step.
    pub fn now(&self, tick: u64) -> Option<u32> {
        self.frames.get(&tick).copied().flatten()
    }

    /// The recorded clock of the latest drawn tick at or before `tick`
    /// that has one.
    pub fn now_at_or_before(&self, tick: u64) -> Option<u32> {
        self.frames.range(..=tick).rev().find_map(|(_, v)| *v)
    }
}

/// A `ready` cell of the frame schedule: `none`, or
/// `class:mask` pairs (mask in hex) separated by commas.
fn parse_ready(v: &str) -> Option<crate::bridge::world::GfxReady> {
    let mut r = crate::bridge::world::GfxReady::default();
    if v == "none" {
        return Some(r);
    }
    for pair in v.split(',') {
        let (c, m) = pair.split_once(':')?;
        r.masks
            .insert(c.parse().ok()?, u32::from_str_radix(m, 16).ok()?);
    }
    Some(r)
}

/// `play --dump-draws`: the rendering facts of
/// the first drawn frame at or after the requested server tick, then the
/// app exits. Reads the built frame only.
#[derive(Resource)]
pub struct DrawDump {
    pub request: crate::facts::export::DumpRequest,
    /// Frames drawn so far (the dump's `seq`).
    seen: u64,
    /// The next dump of `request.at_ticks`; all done at its length.
    next: usize,
    /// Draws made with the Esc menu open at a paused-loop tick.
    menu_draws: u32,
}

impl DrawDump {
    pub fn new(request: crate::facts::export::DumpRequest) -> Self {
        DrawDump {
            request,
            seen: 0,
            next: 0,
            menu_draws: 0,
        }
    }

    fn done(&self) -> bool {
        self.next >= self.request.at_ticks.len()
    }
}

/// What the last presented frame held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameStats {
    pub bridge_frame: u64,
    /// The server tick the frame shows (`ClientWorld::server_ticks`).
    pub server_tick: u64,
    pub items: usize,
    pub units_drawn: usize,
    pub units_hidden: usize,
    pub ui_sent: usize,
    pub ui_unhandled: usize,
    pub gpu: bool,
}

/// The UI core (non-send: panels are plain trait objects). Optional: with
/// none inserted the frame has no UI items.
pub struct WorldViewUi {
    pub root: UiRoot,
    pub strings: Box<dyn StringLookup>,
    pub queue: UiQueue,
    /// The game's UI (`OriginalUi`) whose panels are in `root`: it
    /// applies their outputs, and its open mode is the feed's
    /// ([`ViewFeed::set_ui_open_mode`]).
    pub original: Option<OriginalUi>,
    /// Key bindings: pressed keys become [`UiEvent::Action`]s.
    pub bindings: Option<Bindings>,
    /// Makes the panel DC6 files of the frame's UI draws resident.
    pub art: Option<PanelArtLoader>,
    /// Makes the UI text fonts (`.tbl` and glyph DC6) of the frame's text
    /// draws resident (`ui_bind::TextAssetLoader`).
    pub text: Option<TextAssetLoader>,
    /// Last cursor position sent, so moves are reported once.
    cursor: Option<FramePos>,
    /// The last cursor position inside the frame: a button released
    /// outside the frame is released there.
    last_at: crate::ui::Point,
    /// The window lost the focus since the last pass: the held world
    /// buttons are released, so no click stays held while the game is in
    /// the background.
    focus_lost: bool,
    /// A `play --input` script drives the pointer (no window pointer): a
    /// cursor jump then queues the mouse move that moving the OS cursor
    /// would cause, instead of moving the window cursor.
    scripted: bool,
    /// The cursor the last drawn frame hands to the window
    /// ([`super::hw_cursor`]).
    pub hw_cursor: super::hw_cursor::Pending,
}

impl WorldViewUi {
    pub fn new(root: UiRoot, strings: Box<dyn StringLookup>) -> Self {
        WorldViewUi {
            root,
            strings,
            queue: UiQueue::default(),
            original: None,
            bindings: None,
            art: None,
            text: None,
            cursor: None,
            last_at: crate::ui::Point::new(0, 0),
            focus_lost: false,
            scripted: false,
            hw_cursor: Default::default(),
        }
    }

    /// The cursor position last reported to the UI (the click view's
    /// mouse).
    pub fn cursor(&self) -> Option<FramePos> {
        self.cursor
    }

    /// Sets the cursor position a headless driver reported with its own
    /// `CursorMoved` (`app::autoplay_host`, the window's `ui_input` path).
    pub fn set_cursor(&mut self, at: FramePos) {
        self.cursor = Some(at);
        if let FramePos::Inside(p) = at {
            self.last_at = p;
        }
    }
}

/// Sound requests for the audio frame, in order: the bridge outputs'
/// (S→C 0x2C server sounds, the sounds of the UI dispatch of 0x5D /
/// 0x77) and the UI's own (clicks, panel open and close). The output
/// dispatcher and the world view append; the audio side
/// drains.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct UiSounds(pub Vec<SoundRequest>);

/// The GPU path's main-world half: the atlas of the frame store, and the
/// pages last handed to the node (when frames are added, only the pages
/// they were packed into are copied; the rest stay shared). The compute
/// compositor itself runs in the render world ([`super::node`]).
#[derive(Resource)]
pub struct WorldViewGpu {
    pub atlas: GpuAtlas,
    pages: Arc<Vec<Arc<AtlasPage>>>,
    /// The atlas version `pages` were shared at ([`GpuAtlas::version`]).
    held: u64,
    /// Jobs handed to the node.
    seq: u64,
    /// The job whose indices are not committed to the cycle yet, and its
    /// plan.
    pending: Option<(u64, FramePlan)>,
    /// Ticks in a row that waited for `pending` ([`NODE_STALL_TICKS`]).
    stalled: u32,
    /// The act's blend tables as the native UI pass blends them.
    ui_tables: super::native_ui::TableKindCache,
}

impl WorldViewGpu {
    /// No composed job is waiting to be read back into the index cycle.
    pub fn idle(&self) -> bool {
        self.pending.is_none()
    }

    /// A fresh GPU path: an empty atlas of [`GPU_ATLAS_PAGES`] pages, no
    /// job handed out yet. A headless host may insert one to run the
    /// main-world half of the GPU path (atlas, packing, jobs) without a
    /// render world.
    pub fn new() -> std::result::Result<Self, super::ViewError> {
        Ok(WorldViewGpu {
            atlas: GpuAtlas::new(GPU_ATLAS_PAGES)?,
            pages: Arc::new(Vec::new()),
            held: 0,
            seq: 0,
            pending: None,
            stalled: 0,
            ui_tables: Default::default(),
        })
    }
}

/// The presented image and its sprite.
#[derive(Resource)]
pub struct WorldViewTarget {
    pub image: Handle<Image>,
    pub sprite: Entity,
}

#[derive(Component)]
pub(crate) struct WorldViewSprite;

/// Adds the world view systems. `gpu`: add the render-graph node and
/// create [`WorldViewGpu`] once a [`WorldViewState`] exists (no render
/// world, no GPU path: the CPU reference is presented). Add it after
/// Bevy's render plugin.
pub struct WorldViewPlugin {
    pub gpu: bool,
}

impl Default for WorldViewPlugin {
    fn default() -> Self {
        WorldViewPlugin { gpu: true }
    }
}

#[derive(Resource)]
struct GpuWanted(bool);

/// The order of the `PreUpdate` systems that take the bridge after
/// [`bridge_frame`] and before [`mirror_units`]: the frame's outputs
/// first (their C→S answers, 0x31, leave in the frame that made them),
/// then the local player's walk prediction, then the monster tracks
/// (players before monsters, as the client update pass orders units).
/// Pinned so the
/// order of sends within a frame is not left to Bevy's executor.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreviewOrder {
    Outputs,
    PlayerWalk,
    MonsterWalk,
}

/// Configures [`PreviewOrder`] in `PreUpdate` (idempotent).
pub fn configure_preview_order(app: &mut App) {
    app.configure_sets(
        PreUpdate,
        (
            PreviewOrder::Outputs,
            PreviewOrder::PlayerWalk,
            PreviewOrder::MonsterWalk,
        )
            .chain()
            .after(bridge_frame)
            .before(mirror_units),
    );
}

/// The paused pass's input: the bridge (and with it the single-player
/// game) is paused while the game's UI has state 9 (the Esc menu) or 11
/// open.
pub fn pause_frame(mut bridge: ResMut<BridgeResource>, ui: Option<NonSend<WorldViewUi>>) {
    let paused = ui
        .as_ref()
        .and_then(|u| u.original.as_ref())
        .is_some_and(|o| o.is_open(9) || o.is_open(11));
    bridge.0.set_paused(paused);
}

/// The pending action's step: after the
/// pass's input, before the server tick, in the pass whose bridge frame
/// ticks. A pass runs the world view's input (`Update`) and then the next
/// frame's [`bridge_frame`] (`PreUpdate`), so the step runs here, before a
/// bridge frame that is not held or paused, at most once per tick
/// ([`super::interact::PreviewInteract::frame`]). Running it in the
/// tick's first pass instead would make the step of a click in the tick's
/// last pass wait a tick, so the click's 0x01 and its first 0x02 would
/// reach the server one tick apart instead of together, and a walk would
/// turn a repeat early.
fn pending_frame(
    mut bridge: ResMut<BridgeResource>,
    mut state: ResMut<WorldViewState>,
    walk: Option<Res<PreviewWalk>>,
    drawn: Option<Res<crate::bridge::mirror::DrawnTick>>,
    input_only: Option<Res<crate::bridge::mirror::InputOnlyPass>>,
) -> Result {
    let Some(w) = walk else {
        return Ok(());
    };
    let tick = bridge.0.world().server_ticks;
    if tick == 0
        || bridge.0.is_paused()
        || crate::bridge::mirror::held(&bridge.0, drawn.as_deref(), input_only.as_deref())
    {
        return Ok(());
    }
    let walking = w.predict.walking().is_some();
    let run = w.run.word() & crate::controls::click::mods::RUN != 0;
    let local_at = bridge
        .0
        .world()
        .local()
        .and_then(|p| p.precise)
        .or_else(|| w.predict.position());
    for o in state
        .interact
        .frame(&mut bridge.0, walking, tick, run, local_at)?
    {
        debug!("interact: {o:?}");
    }
    Ok(())
}

impl Plugin for WorldViewPlugin {
    fn build(&self, app: &mut App) {
        // The sharp present material needs the renderer; without one (a
        // headless run) the frame stays a plain sprite.
        super::sharp::add_sharp(app);
        super::native_ui::add_native_ui(app);
        let node = self.gpu && add_node(app);
        configure_preview_order(app);
        app.insert_resource(GpuWanted(node))
            .init_resource::<UiSounds>()
            .init_resource::<crate::bridge::mirror::InputOnlyPass>()
            .init_resource::<super::smooth::SmoothCamera>()
            .init_resource::<super::live_view::AdaptiveView>()
            .init_resource::<super::unit_motion::UnitMotion>()
            .init_resource::<super::unit_motion::MotionShown>()
            .init_resource::<super::hw_cursor::HardwareCursorShown>()
            .add_systems(
                PreUpdate,
                (
                    pause_frame
                        .before(bridge_frame)
                        .run_if(resource_exists::<BridgeResource>),
                    super::live_view::live_view
                        .after(pause_frame)
                        .before(pending_frame)
                        .before(bridge_frame)
                        .run_if(resource_exists::<WorldViewState>),
                    pending_frame
                        .after(super::live_view::live_view)
                        .before(bridge_frame)
                        .run_if(resource_exists::<BridgeResource>)
                        .run_if(resource_exists::<WorldViewState>),
                    deliver_outputs
                        .in_set(PreviewOrder::Outputs)
                        .run_if(resource_exists::<BridgeResource>),
                ),
            )
            .add_systems(
                Update,
                (
                    init_gpu,
                    ui_input,
                    script_input,
                    world_view_frame.pipe(skip_failed_frame),
                    present_sharp.run_if(resource_exists::<Assets<SharpMaterial>>),
                    super::hw_cursor::apply,
                    present_scale,
                    super::smooth::observe_camera,
                    super::unit_motion::compose_motion,
                    super::smooth::present_layers,
                    super::smooth::present_native,
                )
                    .chain()
                    .run_if(resource_exists::<BridgeResource>)
                    .run_if(resource_exists::<WorldViewState>),
            );
    }
}

/// One input of the effect layer from a delivered frame, in list order.
pub enum EffectIn<'a> {
    /// An effect output.
    Output(&'a Output),
    /// The overlay calls the UI made while applying one UI output.
    Overlays(Vec<crate::ui::original::msg_ui::OverlayCall>),
}

/// What the output dispatcher could not apply.
#[derive(Debug, thiserror::Error)]
pub enum DeliverError {
    #[error(transparent)]
    Ui(#[from] crate::ui::original::OriginalUiError),
    /// The bridge could not send the answer of 0x28's dialog branch.
    #[error(transparent)]
    Bridge(#[from] BridgeError),
}

/// The output dispatcher system: the last bridge frame's outputs through
/// [`deliver`]; the sound requests are appended to [`UiSounds`]. Without
/// the game's UI the UI outputs
/// have no consumer and are dropped with a log line.
pub fn deliver_outputs(
    mut bridge: ResMut<BridgeResource>,
    outputs: Option<ResMut<FrameOutputs>>,
    ui: Option<NonSendMut<WorldViewUi>>,
    sounds: Option<ResMut<UiSounds>>,
    state: Option<ResMut<WorldViewState>>,
) -> Result {
    // The overlays the model creates for hits go to the effect layer with
    // the UI's calls.
    let mut state = state;
    let hits = bridge.0.take_hit_overlays();
    if let Some(s) = state.as_mut() {
        if !hits.is_empty() {
            s.missiles.hit_overlays(hits);
        }
    }
    let Some(mut outputs) = outputs else {
        return Ok(());
    };
    let list = std::mem::take(&mut outputs.0);
    if list.is_empty() {
        return Ok(());
    }
    let original = ui.map(|u| u.into_inner()).and_then(|u| u.original.as_mut());
    let preview = state.as_ref().is_some_and(|s| s.preview);
    let tick = bridge.0.world().server_ticks;
    // The effect outputs go to the effect layer in list order, each
    // queued at the tick of its delivery. The UI's overlay calls join
    // them at the output that made them: a message's overlays are created
    // when it is received, in message order.
    let mut effects = |e: EffectIn<'_>| match e {
        EffectIn::Output(o) => {
            if !state
                .as_mut()
                .is_some_and(|s| s.missiles.effect_output(tick, o))
            {
                debug!("effects output {o:?}: no effect consumer yet");
            }
        }
        EffectIn::Overlays(calls) => match state.as_mut() {
            Some(s) => s.missiles.overlay_calls(tick, calls),
            None => debug!("ui overlay calls {calls:?}: no effect layer"),
        },
    };
    let requests = deliver_with(&mut bridge.0, &list, original, preview, Some(&mut effects))?;
    if let Some(mut s) = sounds {
        s.0.extend(requests);
    }
    Ok(())
}

/// Applies one bridge frame's outputs in list order: UI outputs to the
/// game's UI (its sounds appended at
/// once, so the request order is the call order), sound outputs as
/// requests, effect outputs to no consumer ([`deliver_with`] takes one).
/// Returns the sound requests in order.
///
/// When the UI chose the case of 0x28's dialog branch
/// ([`OriginalUi::take_dialog_answer`]), the bridge applies it at once,
/// before the next output: its model writes, then C→S 0x31 in the slot after
/// 0x28's 0x2F and the sends that waited behind it, in the same frame as
/// the 0x28 (an unanswered slot is dropped at the next bridge frame).
pub fn deliver<L: ServerLink>(
    bridge: &mut Bridge<L>,
    list: &[Output],
    original: Option<&mut OriginalUi>,
) -> Result<Vec<SoundRequest>, DeliverError> {
    deliver_with(bridge, list, original, false, None)
}

/// [`deliver`], with the play preview's chat close after the dialog
/// branch when `preview_chat_end` (`bridge::chat_end`), and the effect
/// outputs given in list order to `effects`,
/// each UI output's overlay calls given there right after it
/// (none: dropped).
pub fn deliver_with<L: ServerLink>(
    bridge: &mut Bridge<L>,
    list: &[Output],
    mut original: Option<&mut OriginalUi>,
    preview_chat_end: bool,
    effects: Option<&mut dyn FnMut(EffectIn<'_>)>,
) -> Result<Vec<SoundRequest>, DeliverError> {
    let requests = std::cell::RefCell::new(Vec::new());
    let effects = std::cell::RefCell::new(effects);
    let bridge = std::cell::RefCell::new(bridge);
    dispatch::<DeliverError>(
        list,
        &mut |o| {
            match original.as_deref_mut() {
                Some(ui) => {
                    ui.apply_output(o, bridge.borrow().world())?;
                    requests.borrow_mut().extend(ui.take_sounds());
                    // S→C 0x5A: the codes whose line goes to the screen
                    // message list, which plays UI sound 6 even for the
                    // empty line of the local player's own join (code 2).
                    // The game's UI does not build these lines yet, so the
                    // sound is requested here ([`event_text_adds_line`]).
                    if let Output::EventText { bytes, .. } = o {
                        if event_text_adds_line(bytes[1]) {
                            requests.borrow_mut().push(SoundRequest::Ui(6));
                        }
                    }
                    if let Some(f) = effects.borrow_mut().as_mut() {
                        let calls = ui.take_overlay_calls();
                        if !calls.is_empty() {
                            f(EffectIn::Overlays(calls));
                        }
                    }
                    for s in ui.take_skipped() {
                        debug!("ui output {o:?}: skipped {s}");
                    }
                    if let Some((d, case)) = ui.take_dialog_answer() {
                        bridge.borrow_mut().npc_dialog_branch(&d, case)?;
                        // Preview: no speech, so a dialog of no NPC
                        // interaction closes the chat at once
                        // (`bridge::chat_end`). An NPC interaction keeps the
                        // server state for its greeting and menu.
                        if preview_chat_end && !ui.npc_interaction_up() {
                            bridge.borrow_mut().preview_chat_end(d.guid)?;
                        }
                    }
                }
                None => debug!("ui output {o:?}: no game UI"),
            }
            Ok(())
        },
        &mut |o| {
            match audio_request(o) {
                Some(r) => requests.borrow_mut().push(r),
                None => debug!("audio output {o:?}: no consumer yet"),
            }
            Ok(())
        },
        &mut |o| {
            // The client effect layer.
            match effects.borrow_mut().as_mut() {
                Some(f) => f(EffectIn::Output(o)),
                None => debug!("effects output {o:?}: no effect layer"),
            }
            Ok(())
        },
    )?;
    Ok(requests.into_inner())
}

/// The automap's frame facts: the play frame, the open mode, the unit
/// origin of the frame's one camera; no camera: origin (0, 0).
/// UI state 0x0A: the automap shown.
const AUTOMAP_STATE: u8 = 0x0A;

fn automap_facts(
    camera: Option<&crate::rules::camera::Camera>,
    open_mode: u8,
) -> crate::ui::automap::FrameFacts {
    use crate::rules::camera::FrameSize;
    crate::ui::automap::FrameFacts {
        width: FrameSize::play().width,
        height: FrameSize::play().height,
        open_mode,
        mini_down: false,
        unit_origin: camera.map_or(Default::default(), |c| c.unit),
    }
}

/// The sound request of an audio output, in list order: S→C 0x2C events,
/// the client object code's sounds (mode sounds, requests, player event
/// sounds), the 0x4D shrine sound (played on the player) and the unit
/// frees (a freed unit's sounds stop). `None`: no sound consumer for it.
pub fn audio_request(o: &Output) -> Option<SoundRequest> {
    use crate::bridge::objects::ObjSound;
    Some(match o {
        &Output::ServerSound {
            unit,
            class,
            at,
            event,
        } => SoundRequest::Server {
            unit,
            class,
            at,
            event,
        },
        &Output::UnitFreed { unit, client_only } => SoundRequest::UnitFreed { unit, client_only },
        Output::ObjectSound(ObjSound::Mode {
            unit,
            class,
            mode,
            local_dist,
        }) => SoundRequest::ObjectMode {
            unit: unit.key,
            client_only: unit.client_only,
            class: *class,
            mode: *mode,
            local_dist: *local_dist,
        },
        Output::ObjectSound(ObjSound::Request { id, unit }) => SoundRequest::UnitRequest {
            id: *id,
            unit: unit.key,
        },
        &Output::ObjectSound(ObjSound::PlayerEvent { player, event }) => {
            SoundRequest::PlayerEvent {
                unit: player,
                event: u16::from(event),
            }
        }
        &Output::ShrineSound { sound, player } => SoundRequest::UnitRequest {
            id: sound as i32,
            unit: player,
        },
        &Output::MissileSound(m) => {
            use crate::bridge::client_missiles::MissileSound as M;
            match m {
                M::Request { id, missile } => SoundRequest::UnitRequest { id, unit: missile },
                M::StopOwnerGroup { owner, id } => SoundRequest::GroupStop { unit: owner, id },
                M::DetachTravel { missile, id } => SoundRequest::GroupDetach { unit: missile, id },
                M::DetachHead { missile } => SoundRequest::DetachHead { unit: missile },
            }
        }
        _ => return None,
    })
}

/// Whether a 0x5A code ends in a screen message: codes 0–6, 8–11, 0xD and
/// 0xF–0x12 add a line (0x12 after its screen shake); 7 shows its text
/// another way, and 0xC, 0xE and codes above 0x12 add none.
pub(crate) fn event_text_adds_line(code: u8) -> bool {
    matches!(code, 0..=6 | 8..=11 | 0xD | 0xF..=0x12)
}

/// The player event sounds of a world click's results (`ClickOut::Sound`)
/// as audio requests on the player, in click order.
pub fn click_sounds(player: Option<UnitKey>, outs: &[ClickOut]) -> Vec<SoundRequest> {
    let Some(unit) = player else {
        return Vec::new();
    };
    outs.iter()
        .filter_map(|o| match *o {
            ClickOut::Sound(event) => Some(SoundRequest::PlayerEvent { unit, event }),
            _ => None,
        })
        .collect()
}

/// Creates the GPU path once, when the node exists.
fn init_gpu(
    mut commands: Commands,
    wanted: Res<GpuWanted>,
    state: Res<WorldViewState>,
    mut tried: Local<bool>,
) -> Result {
    if state.is_added() {
        // A new game (`app::session`): its own GPU path.
        *tried = false;
    }
    if *tried || !wanted.0 {
        return Ok(());
    }
    *tried = true;
    commands.insert_resource(WorldViewGpu::new()?);
    Ok(())
}

/// Skill-picker capture precedes ordinary game actions so conflicts cannot
/// open a panel or use a potion while the player is assigning a key.
fn try_bind_skill_key(
    ui: &mut WorldViewUi,
    key: crate::controls::Key,
    world: Option<&crate::bridge::world::ClientWorld>,
    mouse: Option<crate::ui::geom::Point>,
) -> bool {
    use crate::ui::original::SkillKeyResult;
    let (Some(original), Some(bindings), Some(world), Some(mouse)) =
        (ui.original.as_mut(), ui.bindings.as_ref(), world, mouse)
    else {
        return false;
    };
    let ui_mouse = crate::ui::geom::Point::new(
        mouse.x - crate::rules::camera::FrameSize::play().ui_left(),
        mouse.y,
    );
    let path = crate::ui::front_end::screens::controls::config_path();
    match original.bind_skill_key(world, bindings, key, ui_mouse, path.as_deref()) {
        SkillKeyResult::Ignored => false,
        SkillKeyResult::Consumed => true,
        SkillKeyResult::Assigned { action, bindings } => {
            ui.bindings = Some(bindings);
            // The assignment action reads the UI pointer. Deliver its current
            // position first, including when pointer and key arrive together.
            ui.queue.0.push(UiEvent::CursorMoved(mouse));
            ui.cursor = Some(FramePos::Inside(mouse));
            ui.queue
                .0
                .push(UiEvent::Action(crate::ui::ActionId(action.index() as u16)));
            true
        }
    }
}

/// Window input → UI events, on every display frame: the pointer in frame
/// coordinates, every left / right press and release in order
/// ([`super::pointer_input`]), and the bound keys, wheel and extra mouse
/// buttons as actions. The events queue in [`WorldViewUi`] until the next
/// UI pass, so input between game ticks waits for the tick instead of
/// being lost. A button released outside the frame is still released (at
/// the last frame position), and a lost window focus is noted so the held
/// world buttons are released.
#[allow(clippy::too_many_arguments)]
fn ui_input(
    (ui, dev_overlay): (
        Option<NonSendMut<WorldViewUi>>,
        Option<Res<crate::app::dev::overlay::DevOverlay>>,
    ),
    windows: Query<&Window, With<PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    walk: Option<ResMut<PreviewWalk>>,
    script: Option<Res<super::input_script::InputScript>>,
    schedule: Option<Res<FrameSchedule>>,
    focus: Option<Res<Messages<bevy::window::WindowFocused>>>,
    mut focus_cursor: Local<MessageCursor<bevy::window::WindowFocused>>,
    wheel: Option<Res<Messages<bevy::input::mouse::MouseWheel>>>,
    mut wheel_cursor: Local<MessageCursor<bevy::input::mouse::MouseWheel>>,
    mut wheel_acc: Local<crate::controls::original::WheelAccumulator>,
    button_msgs: Option<Res<Messages<bevy::input::mouse::MouseButtonInput>>>,
    mut button_cursor: Local<MessageCursor<bevy::input::mouse::MouseButtonInput>>,
    bridge: Option<Res<BridgeResource>>,
) -> Result {
    // Every left / right press and release of this display frame, in order
    // (`pointer_input`); read first so none waits for a later frame.
    let mut edges = button_msgs
        .as_deref()
        .map(|m| super::pointer_input::edges_from_messages(button_cursor.read(m)))
        .unwrap_or_default();
    if edges.is_empty() {
        edges = super::pointer_input::edges_from_state(&buttons);
    }
    let (Some(mut ui), Ok(window)) = (ui, windows.single()) else {
        return Ok(());
    };
    if dev_overlay.as_ref().is_some_and(|o| o.open) {
        // Input messages are still drained while the dev UI owns the pointer.
        // Release any game button held before opening it, and drop queued
        // clicks so none runs behind the overlay or after it closes.
        ui.focus_lost = true;
        ui.queue.0.clear();
        if let Some(m) = wheel.as_deref() {
            wheel_cursor.read(m).for_each(drop);
        }
        return Ok(());
    }
    if let Some(m) = focus.as_deref() {
        if focus_cursor.read(m).any(|e| !e.focused) {
            ui.focus_lost = true;
        }
    }
    let mouse = match edge::cursor_frame_pos(window).unwrap_or(FramePos::Outside) {
        FramePos::Inside(p) => Some(p),
        FramePos::Outside => None,
    };
    let world = bridge.as_ref().map(|b| b.0.world());
    use crate::controls::{Action, Key};
    // An input is down: a key, or the middle / X buttons.
    let down = |k: Key| {
        let key = keys.as_deref().is_some_and(|ks| {
            edge::KEY_CODES
                .iter()
                .any(|&(c, key)| key == k && ks.pressed(c))
        });
        // `keydown` steps of an input script.
        let scripted = script.as_deref().is_some_and(|s| s.held(k));
        key || scripted
            || match k {
                Key::MouseMiddle => buttons.pressed(MouseButton::Middle),
                Key::Mouse4 => buttons.pressed(MouseButton::Back),
                Key::Mouse5 => buttons.pressed(MouseButton::Forward),
                _ => false,
            }
    };
    // Stand Still (command 36) and Run (command 34) are held while an
    // input bound to them is down (the down handler sets, the up handler
    // clears; Shift, Ctrl and Alt count on both sides).
    // The Run down handler's walk → run switch of a walking player (mode
    // 3, C→S 0x53 / 0x54) is not modelled: the flag reaches the next
    // world click (`RunMods::word`).
    if let (Some(mut walk), Some(bindings)) = (walk, &ui.bindings) {
        let still = edge::action_held(bindings, Action::StandStill, &down);
        if walk.run.stand_still != still {
            walk.run.stand_still = still;
        }
        let run = edge::action_held(bindings, Action::Run, &down);
        if walk.run.run_held != run {
            walk.run.run_held = run;
        }
    }
    // Show Items (command 37): ui 0x0D on while held, off on the release.
    let show = ui
        .bindings
        .as_ref()
        .is_some_and(|b| edge::action_held(b, Action::ShowItems, &down));
    if let Some(o) = ui.original.as_mut() {
        o.set_show_items(show)?;
    }
    // The middle and X buttons and the wheel call their bound command:
    // no panel takes them.
    if let Some(bindings) = ui.bindings.clone() {
        let mut pointer = Vec::new();
        for (b, k) in [
            (MouseButton::Middle, Key::MouseMiddle),
            (MouseButton::Back, Key::Mouse4),
            (MouseButton::Forward, Key::Mouse5),
        ] {
            if buttons.just_pressed(b) {
                pointer.push(k);
            }
        }
        if let Some(m) = wheel.as_deref() {
            for e in wheel_cursor.read(m) {
                // Windows units: 120 per notch.
                let delta = match e.unit {
                    bevy::input::mouse::MouseScrollUnit::Line => (e.y * 120.0) as i32,
                    bevy::input::mouse::MouseScrollUnit::Pixel => e.y as i32,
                };
                pointer.extend(edge::wheel_key(&mut wheel_acc, delta));
            }
        }
        pointer.retain(|&key| !try_bind_skill_key(&mut ui, key, world, mouse));
        let bindings = ui.bindings.as_ref().unwrap_or(&bindings);
        let actions = edge::input_actions(bindings, &pointer);
        ui.queue.0.extend(actions);
    }
    // Shift and Ctrl held, for the panels' modified clicks (e.g. the
    // shift-click to the belt).
    if let (Some(o), Some(keys)) = (ui.original.as_mut(), keys.as_deref()) {
        o.set_shift(keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight));
        o.set_ctrl(keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight));
    }
    // Keys in `KEY_CODES` order, so one frame's actions are ordered the
    // same on every run.
    // The Controls screen takes the raw keys while it is open, and the
    // game's key bindings none (`ui::controls_host`).
    let mut controls_open = false;
    if let (Some(o), Some(keys)) = (ui.original.as_mut(), keys.as_deref()) {
        if o.controls_open() {
            controls_open = true;
            for &(c, _) in edge::KEY_CODES {
                if !keys.just_pressed(c) {
                    continue;
                }
                let vk = match c {
                    KeyCode::Escape => Some(27),
                    _ => edge::key_of(c).and_then(crate::controls::keymap::key_to_vk),
                };
                if let Some(vk) = vk {
                    o.controls_key(vk);
                }
            }
        }
    }
    if let (Some(keys), false) = (keys.as_deref(), controls_open) {
        let mut pressed: Vec<KeyCode> = edge::KEY_CODES
            .iter()
            .map(|&(c, _)| c)
            .filter(|&c| keys.just_pressed(c))
            .collect();
        pressed.retain(|&code| {
            !edge::key_of(code).is_some_and(|key| try_bind_skill_key(&mut ui, key, world, mouse))
        });
        let mode = ui.original.as_ref().map_or(1, |o| o.key_mode());
        let actions = ui.bindings.as_ref().map_or_else(Vec::new, |bindings| {
            edge::key_actions_in_mode(bindings, &pressed, mode)
        });
        ui.queue.0.extend(actions);
        ui.queue.0.extend(edge::key_chars(&pressed));
    }
    // `play --input`: the script owns the pointer (`script_input`). A
    // check run with a frame schedule replays the recorded pointer, the
    // script's or none: the host window's pointer (an Xvfb display's
    // centre) is not an input of the run.
    if script.is_some() || schedule.is_some() {
        return Ok(());
    }
    // A window too small for the frame has no frame mapping: pointer
    // input is dropped as outside the frame.
    let pos = edge::cursor_frame_pos(window).unwrap_or(FramePos::Outside);
    if ui.cursor != Some(pos) {
        let e = match pos {
            FramePos::Inside(p) => UiEvent::CursorMoved(p),
            FramePos::Outside => UiEvent::CursorLeft,
        };
        ui.queue.0.push(e);
        ui.cursor = Some(pos);
    }
    // A press needs the cursor in the frame; a release outside it (a
    // black bar, outside the window) still ends the button, at the last
    // frame position (a left release reads the current mouse anyway).
    let inside = match pos {
        FramePos::Inside(p) => {
            ui.last_at = p;
            true
        }
        FramePos::Outside => false,
    };
    let at = ui.last_at;
    super::pointer_input::pointer_events(&edges, inside, at, &mut ui.queue.0);
    Ok(())
}

/// The window border height (`SM_CYFIXEDFRAME`) of a windowed game on
/// Windows: the part of the border a cursor jump leaves out.
const WINDOW_FRAME_Y: i32 = 3;

/// The mouse move a cursor jump to `at` causes in a windowed game (a
/// panel that opens moves the cursor): the jump places the cursor at
/// `(window left + SM_CXFIXEDFRAME + x, window top + SM_CYCAPTION + y)`,
/// which misses the top border, so the next mouse move reports
/// (x, y − SM_CYFIXEDFRAME); e.g. a jump to (320, 240) reads back as
/// (320, 237).
pub fn cursor_jump_move(at: crate::ui::Point) -> crate::ui::Point {
    crate::ui::Point::new(at.x, at.y - WINDOW_FRAME_Y)
}

/// The tick the script's steps are timed by in the pass of server tick
/// `tick`. In a recorded run the steps of `frame F` are posted at the stop
/// of tick F − 1 and taken at the start of the next client frame, after
/// the frame that ran tick F − 1 has drawn. A D2RUST pass runs the UI
/// events before its draw, so on
/// a tick the schedule draws the script runs one tick behind (its `frame
/// F` steps reach the next pass); on an undrawn tick, and without a
/// schedule, it runs at `tick`.
pub fn script_tick(tick: u64, schedule: Option<&FrameSchedule>) -> u64 {
    match schedule {
        Some(s) if s.drawn(tick) => tick - 1,
        _ => tick,
    }
}

/// Whether a drawn tick's pass is followed by an input-only pass
/// ([`crate::bridge::mirror::InputOnlyPass`]): the schedule draws `tick`
/// and steps of `frame tick + 1` are due. The schedule has them
/// taken after the frame of `tick` drew and before the update of
/// `tick + 1`; a D2RUST pass runs its input before its draw, so the drawn
/// pass leaves them ([`script_tick`]) and, with no pass between, the next
/// pass would run them after its tick `tick + 1`: a hold or click would
/// reach the server a tick late, and every later frame would be one step
/// behind the schedule.
pub fn input_only_after(
    tick: u64,
    schedule: Option<&FrameSchedule>,
    script: &super::input_script::InputScript,
) -> bool {
    schedule.is_some_and(|s| s.drawn(tick)) && script.due_at(tick)
}

/// The frame's cursor step (the animated cursor's frame, which also
/// draws from the client's random seed), once per tick: a tick whose draw waited for the GPU steps it
/// with its weather update, and the later pass that draws that tick does
/// not step it again. With a schedule, a drawn frame without a recorded
/// clock is the last one only (`FrameSchedule::parse`): its step follows
/// the dump.
fn cursor_step(
    ui: Option<&mut WorldViewUi>,
    bridge: &BridgeResource,
    schedule: Option<&FrameSchedule>,
    tick: u64,
    stepped: &mut u64,
    deferred: bool,
) -> std::result::Result<(), super::ViewError> {
    if *stepped == tick || matches!(schedule.map(|s| s.now(tick)), Some(None)) {
        return Ok(());
    }
    *stepped = tick;
    if let Some(o) = ui.and_then(|u| u.original.as_mut()) {
        // a tick drawn more than once: this is its first draw's step
        if let Some(s) = schedule.filter(|s| !s.extra_draws(tick).is_empty()) {
            o.set_host_now(s.first_now(tick));
        }
        if deferred {
            o.hold_cursor_frame(tick);
        }
        o.cursor_step(bridge.0.world())
            .map_err(|e| super::ViewError::Unresolved {
                what: "cursor step",
                spec: "the cursor's animation",
                message: e.to_string(),
            })?;
    }
    Ok(())
}

/// The world draw's seed draws of a pass whose draw waits for the GPU:
/// the floor pass and passes 4 / 9 under the frame's camera, kept for the
/// pass that draws the tick, so the tick's random draws happen once and
/// in order.
fn roll_ahead(
    bridge: &BridgeResource,
    state: &mut WorldViewState,
) -> std::result::Result<(), super::ViewError> {
    let world = bridge.0.world();
    state.feed.prepare(world, &mut state.assets)?;
    let Some((camera, mode)) = super::feed::camera_at(world, state.feed.as_ref(), state.anchor)?
    else {
        return Ok(());
    };
    if super::feed::no_world(mode) {
        return Ok(());
    }
    crate::rules::draw_order::source::roll_ahead(
        world,
        &camera,
        mode,
        state.feed.as_mut(),
        &state.assets,
    )
}

/// The cursor steps of the frames the recorded run drew on a tick after
/// its first one (passes without a client update, after that tick's sound
/// tick), run before the next tick's client update and draw: one forced
/// step each, with that frame's recorded clock.
fn extra_cursor_steps(
    ui: Option<&mut WorldViewUi>,
    bridge: &BridgeResource,
    schedule: Option<&FrameSchedule>,
    tick: u64,
    done: &mut u64,
) -> std::result::Result<(), super::ViewError> {
    let (Some(s), Some(o)) = (schedule, ui.and_then(|u| u.original.as_mut())) else {
        return Ok(());
    };
    let from = (*done + 1).max(1);
    if from >= tick {
        return Ok(());
    }
    *done = tick - 1;
    for t in from..tick {
        for &now in s.extra_draws(t) {
            let Some(now) = now else {
                continue;
            };
            o.force_cursor_step(now);
            o.cursor_step(bridge.0.world())
                .map_err(|e| super::ViewError::Unresolved {
                    what: "cursor step",
                    spec: "the cursor's animation",
                    message: e.to_string(),
                })?;
        }
    }
    if let Some(now) = s.now(tick) {
        o.set_host_now(Some(now));
    }
    Ok(())
}

/// `play --input`: the script's events, once
/// per new server tick, into the UI queue in place of the window pointer.
#[allow(clippy::too_many_arguments)]
fn script_input(
    ui: Option<NonSendMut<WorldViewUi>>,
    mut bridge: ResMut<BridgeResource>,
    script: Option<ResMut<super::input_script::InputScript>>,
    skill_names: Option<Res<super::script_skills::SkillNames>>,
    schedule: Option<Res<FrameSchedule>>,
    input_only: Option<ResMut<crate::bridge::mirror::InputOnlyPass>>,
    drawn: Option<Res<crate::bridge::mirror::DrawnTick>>,
    mut last: Local<u64>,
) {
    if bridge.is_added() {
        *last = 0;
    }
    let (Some(mut ui), Some(mut script)) = (ui, script) else {
        return;
    };
    ui.scripted = true;
    let now = bridge.0.world().server_ticks;
    if now == 0 {
        return;
    }
    let mut input_only = input_only;
    if now == *last && bridge.0.is_paused() {
        // A paused pass (the Esc menu or options open): no server tick,
        // one step of the paced `wkey` / `wclick` steps.
        if let Some(i) = input_only.as_deref_mut() {
            i.0 = false;
        }
        script.pace();
        deliver_script_keys(&mut ui, &mut script, bridge.0.world());
        return;
    }
    let tick = if now == *last {
        // The input-only pass after a drawn tick: the steps of `frame
        // now + 1`, before the server's tick `now + 1`.
        match input_only.as_deref_mut() {
            // Only once the frame of `now` has drawn (the GPU may hold it
            // a pass longer): the steps never reach that frame.
            Some(i) if i.0 && drawn.as_ref().is_none_or(|d| d.0 >= now) => {
                i.0 = false;
                now
            }
            _ => return,
        }
    } else {
        *last = now;
        let t = script_tick(now, schedule.as_deref());
        if let Some(i) = input_only.as_deref_mut() {
            // Decided after this pass's steps are taken (below).
            i.0 = false;
        }
        t
    };
    // The shifted script's clicks come after the update of their pass
    // (`player_anim::step` runs for them in `click::apply`).
    bridge.0.set_late_input(tick != now);
    let next_input_only = |script: &super::input_script::InputScript| {
        tick + 1 == now && input_only_after(now, schedule.as_deref(), script)
    };
    // `clickunit` steps: the unit's screen point from the model camera
    // (open mode 0, no shake; the drawn frame's camera may follow the
    // walk prediction). `skill` / `cast` steps: the skill from the local
    // player's list, by the names of the player's `skills.txt`.
    let world = bridge.0.world();
    // `click ui:ID` steps look the id up among the panels' elements now.
    if script.wants_ui() {
        script.set_ui_elements(super::ui_bind::ui_elements(&ui.root, world, &*ui.strings));
    }
    let cam = super::input_script::script_camera(world, None);
    let names = skill_names.as_deref();
    let events = script.events_full(
        tick,
        &mut |sel| match &cam {
            Some(c) => super::input_script::unit_point(world, c, sel),
            None => Err("no local player".into()),
        },
        &mut |name, hand| super::script_skills::select_skill(world, names, name, hand),
    );
    for n in script.take_notes() {
        warn!("play --input: {n}");
    }
    // The selects leave with this pass's UI frame, as the skill menu's.
    for s in script.take_selects() {
        ui.root.queue_intent(s);
    }
    if next_input_only(&script) {
        if let Some(i) = input_only.as_deref_mut() {
            i.0 = true;
        }
    }
    if !events.is_empty() {
        ui.queue.0.extend(events);
        ui.cursor = script.cursor();
    }
    deliver_script_keys(&mut ui, &mut script, bridge.0.world());
}

/// The script's `key`, `action` and `wclick` steps run this pass, into
/// the UI queue.
fn deliver_script_keys(
    ui: &mut WorldViewUi,
    script: &mut super::input_script::InputScript,
    world: &crate::bridge::world::ClientWorld,
) {
    use super::input_script::KeyInput;
    // `key` steps: the window's key press path (`ui_input`), bound
    // action first, then the typed character, key by key: a frame's
    // keys are separate presses, so an Esc that opens the game menu
    // comes before the arrows and Enter it then takes. `action` steps:
    // the action event a key bound to it sends, in the key modes that
    // run it, in script order with the keys.
    let mode = ui.original.as_ref().map_or(1, |o| o.key_mode());
    for input in script.take_key_inputs() {
        match input {
            KeyInput::Key(k) => {
                let mouse = match ui.cursor {
                    Some(FramePos::Inside(p)) => Some(p),
                    _ => None,
                };
                if try_bind_skill_key(ui, k, Some(world), mouse) {
                    continue;
                }
                let Some(code) = edge::KEY_CODES
                    .iter()
                    .find(|(_, e)| *e == k)
                    .map(|(c, _)| *c)
                else {
                    continue;
                };
                if let Some(bindings) = &ui.bindings {
                    let actions = edge::key_actions_in_mode(bindings, &[code], mode);
                    ui.queue.0.extend(actions);
                }
                ui.queue.0.extend(edge::key_chars(&[code]));
            }
            KeyInput::Action(a) => {
                if crate::controls::keymap::runs_in_key_mode(a, mode) {
                    ui.queue
                        .0
                        .push(UiEvent::Action(crate::ui::ActionId(a.index() as u16)));
                } else {
                    warn!(
                        "play --input: action {}: not run in key mode {mode}",
                        a.name()
                    );
                }
            }
        }
    }
    // `wclick` steps: after this pass's keys.
    let late = script.take_late();
    if !late.is_empty() {
        ui.queue.0.extend(late);
        ui.cursor = script.cursor();
    }
}

/// The world releases of a lost focus: left
/// up (kind 2) while left is held, right up (kind 5) while right is held,
/// each unless `events` already releases that button.
pub fn focus_releases(
    st: &crate::controls::click::ClickState,
    events: &[UiEvent],
    at: crate::ui::Point,
) -> Vec<UiEvent> {
    let released = |b: PointerButton| {
        events
            .iter()
            .any(|e| matches!(e, UiEvent::Release { button, .. } if *button == b))
    };
    [
        (st.left_held, PointerButton::Left),
        (st.right_held, PointerButton::Right),
    ]
    .into_iter()
    .filter(|&(held, b)| held && !released(b))
    .map(|(_, button)| UiEvent::Release { button, at })
    .collect()
}

fn rgba_image(rgba: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: play_view().width,
            height: play_view().height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// Milliseconds since the client's first frame: a wall clock for UI
/// animations only, never game logic.
fn wall_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(std::time::Instant::now);
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// A frame the world view could not build is skipped, never the end of
/// the game: the error is logged the first time it is seen and the next
/// frame draws again. A check run (`--dump-draws`, a recorded frame
/// schedule) compares exact frames, so there it still stops the run.
fn skip_failed_frame(
    In(result): In<Result>,
    dump: Option<Res<DrawDump>>,
    schedule: Option<Res<FrameSchedule>>,
    mut state: ResMut<WorldViewState>,
    mut seen: Local<std::collections::BTreeSet<String>>,
) {
    let Err(e) = result else {
        return;
    };
    if dump.is_some() || schedule.is_some() {
        panic!("world view frame: {e}");
    }
    state.skipped_frames += 1;
    let message = e.to_string();
    if seen.len() < 256 && seen.insert(message.clone()) {
        warn!("world view: frame skipped: {message}");
    }
}

/// UI input and world clicks every client loop pass (the held repeat and
/// the per-pass latch run per pass, not per server tick); draw list,
/// composition and image update once per presented server tick. Before
/// the first tick nothing runs.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn world_view_frame(
    mut commands: Commands,
    mut bridge: ResMut<BridgeResource>,
    mut state: ResMut<WorldViewState>,
    mut ui: Option<NonSendMut<WorldViewUi>>,
    mut gpu: Option<ResMut<WorldViewGpu>>,
    // The node's read-back indices, the smooth-camera layer targets
    // (`super::smooth`), the native UI pass's targets
    // (`super::native_ui`) and a headless host's scripted clock, grouped to
    // stay within a system's parameter count.
    (indices, composed, layers, native, config, scripted, unit_art): (
        Option<Res<NodeIndices>>,
        Option<Res<NodeComposed>>,
        Option<Res<super::smooth::SmoothLayers>>,
        Option<Res<super::native_ui::NativeUiTargets>>,
        Option<Res<crate::app::config::ConfigRes>>,
        Option<Res<crate::bridge::mirror::ScriptedClock>>,
        Option<Res<crate::app::ui::UnitArt>>,
    ),
    target: Option<Res<WorldViewTarget>>,
    mut images: ResMut<Assets<Image>>,
    mut sounds: crate::app::sound::FrameSounds,
    mut walk: Option<ResMut<PreviewWalk>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut dump: Option<ResMut<DrawDump>>,
    mut drawn: Option<ResMut<crate::bridge::mirror::DrawnTick>>,
    mut exit: MessageWriter<AppExit>,
    mut schedule: Option<ResMut<FrameSchedule>>,
    mut cursor_ticks: Local<(u64, u64)>,
) -> Result {
    if bridge.is_added() {
        *cursor_ticks = (0, 0);
    }
    let tick = bridge.0.world().server_ticks;
    if tick == 0 {
        return Ok(());
    }
    // The extra draws of the ticks before this one come before this tick's
    // update (and its sound tick), drawn or not.
    if schedule.as_ref().is_some_and(|s| s.started) {
        extra_cursor_steps(
            ui.as_deref_mut(),
            &bridge,
            schedule.as_deref(),
            tick,
            &mut cursor_ticks.1,
        )?;
    }
    // A check run's recorded frame schedule: a tick the recorded run drew
    // no frame on is a client update without a frame here too: no draw,
    // no weather update, no cursor step; the pass's input (UI events,
    // world clicks) still runs, before the next tick's update.
    let mut skipped = false;
    if let Some(s) = schedule.as_deref_mut() {
        if !s.started {
            s.started = true;
            state.feed.follow_frame_schedule();
            // The monster graphics-ready flags the draw and the dead
            // snap's write-back read: a recorded host input.
            bridge
                .0
                .set_gfx_ready((!s.ready.is_empty()).then(|| std::sync::Arc::new(s.ready.clone())));
            if let Some(o) = ui.as_mut().and_then(|u| u.original.as_mut()) {
                o.set_cursor_timers(s.cursor_last, s.cursor_idle);
            }
        }
        if let Some(o) = ui.as_mut().and_then(|u| u.original.as_mut()) {
            o.set_host_now(s.now(tick));
        }
        // The client sender's duplicate filter reads the host clock too:
        // the recorded clock of the latest
        // drawn tick (a tick without a frame keeps the one before).
        if let Some(t) = s.now_at_or_before(tick) {
            bridge.0.set_send_now(Some(t));
        }
        skipped = !s.drawn(tick);
    }
    // Live play (no recorded schedule, no dump): the Esc menu's pentagrams
    // and the UI timers (`crate::ui::ui_clock`) run on the wall clock; a
    // headless host (repro, autoplay, soak) gives its scripted step clock
    // instead, so its runs do not depend on how fast the machine is.
    let live = schedule.is_none() && dump.is_none();
    let wall = || {
        scripted
            .as_ref()
            .map_or_else(wall_ms, |c| u64::from(c.now()))
    };
    let paused_menu = ui
        .as_ref()
        .and_then(|u| u.original.as_ref())
        .is_some_and(|o| o.is_open(9) || o.is_open(11));
    if let Some(o) = ui.as_mut().and_then(|u| u.original.as_mut()) {
        o.set_menu_wall_ms(live.then(wall));
    }
    // A new server tick is drawn; the previous GPU frame is the base of
    // this one: commit its indices first, or draw on a later pass.
    let mut draw = !skipped && !state.last.is_some_and(|l| l.server_tick == tick);
    // The paused loop draws again and again without a tick: a dump of
    // such a tick is the second draw with the Esc menu open
    // (the first steps the pentagram), so those are drawn on the same tick.
    if !draw && !skipped {
        let paused_loop = schedule
            .as_ref()
            .and_then(|s| s.draws.get(&tick))
            .is_some_and(|v| v.len() > 1);
        let menu_open = ui
            .as_ref()
            .and_then(|u| u.original.as_ref())
            .is_some_and(|o| o.is_open(9));
        let menu_draws = dump
            .as_ref()
            .filter(|d| !d.done())
            .map_or(2, |d| d.menu_draws);
        draw = paused_loop && menu_open && menu_draws < 2;
        // Live play: the paused loop draws every pass, so the open menu
        // follows the
        // pointer and keys and its pentagrams spin while no tick runs.
        if live && bridge.0.is_paused() && paused_menu {
            draw = true;
        }
        // Every draw the schedule lists between the first and the last of
        // the tick steps the cursor before the last one is drawn.
        if draw && menu_draws == 1 {
            if let (Some(s), Some(o)) = (
                schedule.as_deref(),
                ui.as_mut().and_then(|u| u.original.as_mut()),
            ) {
                for &now in s.extra_draws(tick) {
                    let Some(now) = now else {
                        continue;
                    };
                    o.force_cursor_step(now);
                    o.cursor_step(bridge.0.world())
                        .map_err(|e| super::ViewError::Unresolved {
                            what: "cursor step",
                            spec: "the cursor's animation",
                            message: e.to_string(),
                        })?;
                }
            }
        }
    }
    // In live play the indices of the last job come back a frame late (the
    // render world composes it while this frame runs): once that job is
    // composed, this frame is composed over the GPU's own copy of them
    // instead of waiting a frame (`super::node`). A check run or a dump
    // keeps the exact cycle: it waits for every read-back.
    let chain = live && drawn.is_none();
    let mut base_after = None;
    if let (true, Some(g)) = (draw, gpu.as_deref_mut()) {
        if let Some((seq, plan)) = g.pending {
            match indices.as_ref().and_then(|i| i.take(seq)) {
                Some(back) => {
                    state.cycle.commit(plan, back)?;
                    g.pending = None;
                    g.stalled = 0;
                }
                None if chain
                    && composed
                        .as_ref()
                        .is_some_and(|c| c.wait_for(seq, super::node::CHAIN_WAIT)) =>
                {
                    state.cycle.pass(plan)?;
                    g.pending = None;
                    g.stalled = 0;
                    base_after = Some(seq);
                }
                // The render world has not composed the job for
                // `NODE_STALL_TICKS` ticks: it failed it (or keeps failing
                // it) and would never answer, so waiting longer freezes
                // the picture for good. In live play the job is given up:
                // its frame counts as passed and the next tick's job is
                // composed over the packed base, a frame that may be a
                // little off rather than none.
                None if chain && g.stalled + 1 >= NODE_STALL_TICKS => {
                    warn!(
                        "world view: compose job {seq} not composed after {NODE_STALL_TICKS} ticks; drawing on"
                    );
                    state.cycle.pass(plan)?;
                    g.pending = None;
                    g.stalled = 0;
                }
                None => {
                    g.stalled += 1;
                    draw = false;
                }
            }
        }
    }
    let state = &mut *state;
    // The local player's position and the shake are decided once per
    // drawn tick (like the draw: the shake
    // draws the player seed); the UI (overhead text), the pick, the labels,
    // the automap and the world draw all read this one, and the passes
    // between ticks keep the drawn tick's. An error here is the frame
    // build's own, reported there.
    // The input pump (the clicks) runs before the update and the draw of
    // this tick: it reads the camera of the last drawn frame.
    let click_anchor = state.anchor;
    if draw && state.rolled_tick != tick {
        state.anchor =
            super::feed::frame_anchor(bridge.0.world(), state.feed.as_mut()).unwrap_or_default();
    }
    let mut anchor = state.anchor;
    // The frame's one camera, once the UI has set this frame's open mode.
    let mut placed = None;
    let ui_frame = match ui.as_mut() {
        Some(ui) => {
            let ui = &mut **ui;
            if let Some(o) = ui.original.as_mut() {
                o.set_frame_anchor(anchor);
                o.set_palette(&state.assets.palette);
                // A recorded host message whose clock is at or before this
                // frame's own was handled in the pump before the frame: it
                // shows on this frame. A later one shows on the next
                // (below).
                let early = schedule.as_deref().and_then(|s| {
                    let idle = *s.idle.get(&tick)?;
                    (idle <= s.now(tick)?).then_some(idle)
                });
                if let (true, Some(idle)) = (draw, early) {
                    o.host_cursor_message(idle);
                }
            }
            let mut frame = run_ui_with(
                &mut ui.root,
                &mut ui.queue,
                &mut bridge.0,
                ui.strings.as_ref(),
                ui.original.as_mut(),
            )?;
            // A recorded host window message before this frame (the
            // schedule's `cursor_idle`).
            if let (Some(o), Some(&idle)) = (
                ui.original.as_mut(),
                schedule.as_deref().and_then(|s| s.idle.get(&tick)),
            ) {
                if draw {
                    o.host_cursor_message(idle);
                }
            }
            if let Some(original) = ui.original.as_mut() {
                // A panel's cursor jump ([`cursor_jump_move`]): the window
                // cursor moves; a scripted pointer gets the
                // mouse move that move causes at the next pass.
                let warp = original.take_cursor_warp();
                let mut host_moved = None;
                match (warp, ui.scripted) {
                    (Some(at), true) => {
                        let moved = cursor_jump_move(at);
                        let host_timed = schedule.as_deref().is_some_and(|s| !s.idle.is_empty());
                        original.cursor_jump_moved(at, moved, bridge.0.world(), host_timed);
                        if host_timed {
                            host_moved = Some(moved);
                        }
                        ui.queue.0.push(UiEvent::CursorMoved(moved));
                        ui.cursor = Some(FramePos::Inside(moved));
                    }
                    (Some(at), false) => {
                        if let Ok(mut window) = windows.single_mut() {
                            if let Ok(pos) = edge::frame_to_window(&window, at) {
                                window.set_physical_cursor_position(Some(pos.as_dvec2()));
                            }
                        }
                    }
                    (None, _) => {}
                }
                // A check run replays the pointer the OS reported after the
                // jump (the recorded host input) when D2RUST's own jump agrees
                // on x; a different x is left to show in the cursor row.
                if let (Some(at), Some(s)) = (warp, schedule.as_deref()) {
                    if let Some((_, &(x, y))) = s.pointer.range(tick + 1..).next() {
                        let p = crate::ui::Point::new(x, y);
                        if host_moved == Some(p) {
                            // The jump's own move, left to the recorded
                            // host message.
                        } else if x == at.x {
                            ui.queue.0.push(UiEvent::CursorMoved(p));
                            ui.cursor = Some(FramePos::Inside(p));
                        } else {
                            warn!("cursor jump: D2RUST x {} vs recorded {x}", at.x);
                        }
                    }
                }
                let outcome = original.take_outcome();
                for e in &outcome.effects {
                    debug!("ui: {e:?}");
                }
                if let Some(s) = sounds.ui.as_deref_mut() {
                    s.0.extend(outcome.sounds);
                }
                state.feed.set_ui_open_mode(original.open_mode());
                // The Esc menu's "Save and Exit Game": C→S 0x69; the app
                // ends on the server's answer, so the save is written first.
                if original.take_exit_request() {
                    if let Err(e) = crate::app::save::request_save_and_exit(&mut bridge.0) {
                        warn!("save and exit: {e}");
                    }
                }
                // Configure Controls over the game (`ui::controls_host`).
                let expansion = original.expansion_installed();
                original.service_controls(
                    expansion,
                    crate::ui::front_end::screens::controls::config_path(),
                );
                if let Some(b) = original.take_accepted_bindings() {
                    ui.bindings = Some(b);
                }
                if let Some(b) = &ui.bindings {
                    original.set_belt_keys(b);
                }
            }
            // The automap's header text, drawn before every panel. Single
            // player: no game name or password; the level name is the
            // string of the player's level.
            if let (Some(a), Some(o)) = (state.automap.as_ref(), ui.original.as_ref()) {
                let world = bridge.0.world();
                let strings = ui.strings.as_ref();
                let level = world
                    .local_player
                    .and_then(|k| crate::bridge::modes::level_of(world, k))
                    .and_then(|id| o.level_name_key(id))
                    .and_then(|key| strings.get(&key).map(<[u16]>::to_vec))
                    .unwrap_or_default();
                let facts = crate::ui::automap::header::HeaderFacts {
                    level_name: level,
                    difficulty: world.difficulty,
                    expansion: world.expansion != 0,
                    ..Default::default()
                };
                let fonts = o.font_measure();
                let draws = super::automap_view::header_draws(
                    a,
                    world,
                    o.open_mode(),
                    &facts,
                    &|id| strings.get_id(id).map(<[u16]>::to_vec),
                    &|font, t| fonts.as_ref().and_then(|f| f.width_a(font, t)),
                );
                frame.draws.splice(0..0, draws);
            }
            if let (true, Some(art)) = (draw, &ui.art) {
                art.ensure(&frame.draws, &mut state.assets)?;
            }
            if let (true, Some(text)) = (draw, &ui.text) {
                text.ensure(&frame.draws, &mut state.assets)?;
            }
            let mouse = match ui.cursor {
                Some(FramePos::Inside(p)) => (p.x, p.y),
                _ => (0, 0),
            };
            // The real mouse's cursor, and an item held on it, move at
            // display rate as the window's hardware cursor (`hw_cursor`);
            // scripted and scheduled runs keep both in the frame.
            if let (true, false, None, Some(art)) = (draw, ui.scripted, schedule.as_ref(), &ui.art)
            {
                let item_drawn = ui.original.as_ref().is_some_and(|o| o.cursor_item_drawn());
                ui.hw_cursor = super::hw_cursor::take_cursor(
                    &mut frame.draws,
                    &art.files,
                    state.rules.as_ref(),
                    &state.assets,
                    &state.assets.palette,
                    mouse,
                    item_drawn,
                );
            }
            // The hover pick runs once per client update, drawn or not, and
            // the press pumped before the next update reads that pick. A pass that does not draw takes its tick's update and then
            // the clicks of the next pump (the script times them so), so its
            // pick runs first, under the last drawn frame's camera and anchor.
            let tick = bridge.0.world().server_ticks;
            if state.preview && !draw && tick != state.pick_tick {
                state.pick_tick = tick;
                let last_cam = *state.camera.read().unwrap_or_else(|e| e.into_inner());
                state.rect_hover = match (pick_point(hover_pointer(ui, mouse), &ui.root), last_cam)
                {
                    (Some(m), Some(c)) => tick_pick(
                        bridge.0.world(),
                        &*state,
                        c,
                        m,
                        state.anchor.map(|a| {
                            let p = a.player.client();
                            (p.x, p.y)
                        }),
                    ),
                    _ => None,
                };
                state.prev_hover = state.rect_hover;
            }
            let view = crate::bridge::click::ClickView {
                size: crate::rules::camera::FrameSize::play(),
                open_mode: ui.original.as_ref().map_or(0, |o| o.open_mode().get()),
                // The right panels end 40 px above the frame's bottom.
                right_panel_bottom: crate::rules::camera::FrameSize::play().play_height(),
                skill_y_limit: crate::rules::camera::FrameSize::play().skill_y_limit(),
                mouse,
                game_menu_open: ui.original.as_ref().is_some_and(|o| o.is_open(9)),
                // The preview's hover pick.
                pick: state.preview,
                // The pick inverts the frame's shaken camera (the anchor
                // decided on a drawn tick).
                shake: click_anchor.map_or((0, 0), |a| a.shake),
                // A press sees the hover the previous pass drew.
                prev_hover: state.preview.then_some(state.prev_hover),
                local_mode: walk.as_deref().and_then(|w| w.predict.mode()),
                speeds: walk.as_deref().and_then(|w| w.speeds),
            };
            // The run lock (command 35) is the toggle action no panel took; the click reads the predicted
            // position and the modifier word.
            let (mods, local_at) = match walk.as_deref_mut() {
                Some(w) => {
                    let toggle = crate::controls::Action::ToggleRun.index() as u16;
                    for e in &frame.unhandled {
                        if *e == UiEvent::Action(crate::ui::ActionId(toggle)) {
                            w.run.toggle_run();
                        }
                    }
                    // The control panel's run button toggles it too.
                    if let Some(o) = ui.original.as_mut() {
                        for _ in 0..o.sync_run(w.run.run_lock) {
                            w.run.toggle_run();
                        }
                        o.sync_run(w.run.run_lock);
                    }
                    (w.run.word(), w.pump_position())
                }
                None => (0, None),
            };
            placed = super::feed::camera_at(bridge.0.world(), state.feed.as_ref(), anchor)?;
            let cam = placed.map(|(c, _)| c);
            // The hover target (the preview's pick under the cursor) is drawn
            // highlighted. A run with no host pointer (a check run) hovers
            // where the UI's own pointer is (it starts at (320, 240)).
            let hover_ptr = hover_pointer(ui, mouse);
            let (over, hover_at) = (hover_ptr.is_some(), hover_ptr.unwrap_or(mouse));
            let hover_mouse = pick_point(hover_ptr, &ui.root);
            // This tick's pick, after the client update and before the
            // draw: this tick's cels under the last drawn frame's camera.
            // The labels and the highlight (the hover target) of this draw
            // read it; the
            // click's `prev_hover` (above) is the pick of the tick before.
            // A pass that does not draw picked before its clicks (above).
            if draw && state.preview {
                state.pick_tick = bridge.0.world().server_ticks;
                let last_cam = *state.camera.read().unwrap_or_else(|e| e.into_inner());
                state.rect_hover = match (hover_mouse, last_cam.or(cam)) {
                    (Some(m), Some(c)) => tick_pick(
                        bridge.0.world(),
                        &*state,
                        c,
                        m,
                        anchor.map(|a| {
                            let p = a.player.client();
                            (p.x, p.y)
                        }),
                    ),
                    _ => None,
                };
                state.prev_hover = state.rect_hover;
            }
            let label_hover = state.rect_hover;
            state
                .feed
                .set_hover(label_hover.filter(|_| over && state.preview && cam.is_some()));
            // The hover pop-up: a hovered monster's or town NPC's name
            // (`world_view::monster_label`), and a hovered object's label
            // (`world_view::object_label`).
            if let Some(c) = cam.as_ref() {
                let fonts = ui.original.as_ref().and_then(|o| o.font_measure());
                let env = super::monster_label::LabelEnv {
                    screen: {
                        let f = crate::scene::Rect::world_clip();
                        (f.width as i32, f.height as i32)
                    },
                    fonts: fonts.as_ref(),
                    free: ui
                        .original
                        .as_ref()
                        .map(|o| o.label_free_screen())
                        .unwrap_or_default(),
                    bar_color: ui.original.as_ref().and_then(|o| o.health_bar_color()),
                };
                let mut labels = state.monster_labels.draw(
                    bridge.0.world(),
                    c,
                    label_hover,
                    ui.strings.as_ref(),
                    &env,
                );
                // A hovered object's label, the same pop-up.
                labels.extend(state.object_labels.draw(
                    bridge.0.world(),
                    c,
                    label_hover,
                    ui.strings.as_ref(),
                    env.screen,
                    env.fonts,
                ));
                // A skill pop-up written later in the UI pass took the one
                // pop-up slot.
                if ui.original.as_ref().is_some_and(|o| o.popup_taken()) {
                    labels.clear();
                }
                if let (true, Some(art)) = (draw, &ui.art) {
                    art.ensure(&labels, &mut state.assets)?;
                }
                if let (true, Some(text)) = (draw, &ui.text) {
                    text.ensure(&labels, &mut state.assets)?;
                }
                // The label draws just before the cursor, the last draw of
                // the UI: it goes in front of the cursor in the list.
                let at = frame.draws.len().saturating_sub(usize::from(matches!(
                    frame.draws.last(),
                    Some(crate::ui::UiDraw::Image(_))
                )));
                frame.draws.splice(at..at, labels);
            }
            // Show Items (ui 0x0D) names every ground item;
            // the label under the pointer takes the pick-up click.
            // A pointer over a panel hovers no label under it either.
            let label_at = if hover_mouse.is_some() {
                hover_at
            } else {
                (i32::MIN, i32::MIN)
            };
            let labels = match (cam.as_ref(), ui.original.as_ref()) {
                (Some(c), Some(o)) if over => o.ground_item_labels(
                    bridge.0.world(),
                    c,
                    label_at,
                    label_hover.filter(|k| k.unit_type == 4).map(|k| k.guid),
                ),
                _ => None,
            };
            let item_labels = labels.as_ref().map(|l| l.draws.clone()).unwrap_or_default();
            state.ground_items.set_labels(labels);
            if !item_labels.is_empty() {
                if let (true, Some(art)) = (draw, &ui.art) {
                    art.ensure(&item_labels, &mut state.assets)?;
                }
                if let (true, Some(text)) = (draw, &ui.text) {
                    text.ensure(&item_labels, &mut state.assets)?;
                }
                let at = frame.draws.len().saturating_sub(usize::from(matches!(
                    frame.draws.last(),
                    Some(crate::ui::UiDraw::Image(_))
                )));
                frame.draws.splice(at..at, item_labels);
            }
            let unhandled =
                state
                    .corpse_clicks
                    .take_clicks(&mut bridge.0, cam.as_ref(), &frame.unhandled)?;
            let unhandled = state.ground_items.take_clicks(&mut bridge.0, &unhandled)?;
            // The belt keys read Shift (feed the hireling) and ui 9 (the
            // Esc menu).
            let belt_facts = ui
                .original
                .as_ref()
                .map(|o| crate::bridge::belt::KeyFacts {
                    shift: o.shift_held(),
                    ui9_open: o.is_open(9),
                })
                .unwrap_or_default();
            crate::bridge::belt::send_keys(&mut bridge.0, &frame.unhandled, belt_facts)?;
            // An input reset (the UI asked for one): held := 0 before this
            // pass's clicks.
            if ui.original.as_mut().is_some_and(|o| o.take_input_reset()) {
                state.click.input_reset();
            }
            // Focus lost: the left / right
            // release of a held button, unless this pass releases it.
            let mut unhandled = unhandled;
            if std::mem::take(&mut ui.focus_lost) {
                let at = crate::ui::Point::new(mouse.0, mouse.1);
                unhandled.extend(focus_releases(&state.click, &unhandled, at));
            }
            let (outs, click_outputs) = world_clicks(
                &mut bridge.0,
                &mut state.click,
                view,
                &unhandled,
                mods,
                local_at,
            )?;
            for o in &outs {
                debug!("world click: {o:?}");
            }
            // The click's walk steps in this tick's update (`PreviewWalk::
            // catch_up`): the position, the anchor and the camera follow.
            if let Some(w) = walk.as_deref_mut() {
                if w.catch_up(bridge.0.world()) {
                    super::walk::publish_prediction(bridge.0.world(), w, state.feed.as_mut());
                    if draw {
                        if let Some(old) = anchor {
                            if let Some(p) = state.feed.player(bridge.0.world())? {
                                state.anchor =
                                    Some(crate::rules::camera::FrameAnchor { player: p, ..old });
                                anchor = state.anchor;
                            }
                        }
                    }
                    placed = super::feed::camera_at(bridge.0.world(), state.feed.as_ref(), anchor)?;
                }
            }
            // The click's sounds (refusals,
            // 0x13 "can't use that in town") and the interact's sound
            // outputs reach the audio layer in click order; the effect
            // outputs have no consumer yet.
            if let Some(s) = sounds.ui.as_deref_mut() {
                s.0.extend(click_sounds(bridge.0.world().local_player, &outs));
                s.0.extend(click_outputs.iter().filter_map(audio_request));
            }
            let pressed = frame
                .unhandled
                .iter()
                .any(|e| matches!(e, UiEvent::Press { .. }));
            if walk.is_some() {
                // The pending action's step runs in [`pending_frame`].
                state.interact.note(&outs, pressed);
            }
            if let (true, Some(w)) = (draw, walk.as_deref()) {
                // Ground items and corpses (their frames count drawn
                // ticks).
                let walking = w.predict.walking().is_some();
                state.ground_items.frame(&mut bridge.0, walking)?;
                state.corpse_clicks.frame(&mut bridge.0, walking)?;
            }
            // The automap toggle: the command no panel took.
            let toggle = crate::controls::Action::ToggleAutomap.index() as u16;
            if let Some(a) = state.automap.as_mut() {
                let tabs = frame
                    .unhandled
                    .iter()
                    .filter(|e| **e == UiEvent::Action(crate::ui::ActionId(toggle)))
                    .count();
                match ui.original.as_mut() {
                    // The command toggles UI state 0x0A, like the
                    // mini-panel Automap button.
                    Some(o) => {
                        for _ in 0..tabs {
                            o.set_ui(u32::from(AUTOMAP_STATE), 2, false)?;
                        }
                    }
                    None => {
                        for _ in 0..tabs {
                            a.toggle(&automap_facts(cam.as_ref(), view.open_mode));
                        }
                    }
                }
                // The automap commands (8–11, 45): F9 re-centre, F10 fade,
                // F11 party, F12 names, V the minimap side.
                let f = automap_facts(cam.as_ref(), view.open_mode);
                let mut spare = crate::ui::automap::options::MemoryStore::default();
                let store: &mut dyn crate::ui::automap::OptionStore =
                    match state.automap_view.as_mut() {
                        Some(v) => v.store_mut(),
                        None => &mut spare,
                    };
                for e in &frame.unhandled {
                    use crate::controls::Action as A;
                    let UiEvent::Action(id) = *e else {
                        continue;
                    };
                    match A::ALL.get(usize::from(id.0)) {
                        Some(A::CenterAutomap) => a.map.centre(&f),
                        Some(A::ToggleAutomapFade) => a.map.options.cycle_fade(store),
                        Some(A::ToggleAutomapParty) => a.map.options.toggle_party(store),
                        Some(A::ToggleAutomapNames) => a.map.options.toggle_party_names(store),
                        Some(A::ToggleMinimap) => a.map.options.toggle_left(store),
                        _ => {}
                    }
                }
                // Space with nothing to close: the automap is cleared, then
                // closed with the rest.
                if let Some(o) = ui.original.as_mut() {
                    if o.take_clear_automap() {
                        a.map.cleared(&f);
                        o.set_ui(u32::from(AUTOMAP_STATE), 1, false)?;
                    }
                    // The shown automap follows UI state 0x0A, whoever set
                    // it (closing re-centres with force 0).
                    if o.is_open(AUTOMAP_STATE) != a.open {
                        a.toggle(&f);
                    }
                }
            }
            // Weapon swap (command 44): no swap while ui 0x0C, 0x17 or
            // 0x19 is open.
            let swap_ok = ui
                .original
                .as_ref()
                .is_none_or(|o| super::swap_key::swap_allowed(&|s| o.is_open(s)));
            if swap_ok {
                super::swap_key::send_swaps(&frame.unhandled, &mut bridge.0)?;
            }
            // The Say commands (27–33, 55): C→S 0x3F with 0x19 + k (0x20
            // for command 55).
            super::swap_key::send_says(&frame.unhandled, &mut bridge.0)?;
            Some(frame)
        }
        None => None,
    };
    if skipped {
        // The pass ran the input (clicks reach the model before the next
        // tick's update, as the input pump does); no draw, weather
        // update or cursor step.
        if let Some(d) = drawn.as_deref_mut() {
            d.0 = tick;
        }
        return Ok(());
    }
    // The pass' client update (its sound requests and unit sounds) comes
    // before its in-game draw's weather update and cursor step: the rolls
    // of both draw on one client seed, so their order decides both.
    sounds.client_update(&bridge, walk.as_deref(), schedule.as_deref())?;
    if !draw {
        // A new tick whose draw waits for the GPU still is a client
        // update: the weather steps. When the bridge waits for
        // the draws (`DrawnTick`), the pass that draws this tick runs its
        // update and cursor step instead: the tick's sound draws (this
        // pass' audio frame) come before its weather update.
        if drawn.is_none() && !state.last.is_some_and(|l| l.server_tick == tick) {
            // The tick's random draws happen in this pass as if it drew:
            // its camera (the shake draws) comes before the weather update,
            // the world draw's floor pass and passes 4 / 9 after it, all
            // before the cursor step and the sound tick. The pass that
            // draws the tick reuses them, so the client seed advances the
            // same whether or not the GPU was free.
            let ahead = state.rolled_tick != tick;
            if ahead {
                state.rolled_tick = tick;
                state.anchor = super::feed::frame_anchor(bridge.0.world(), state.feed.as_mut())
                    .unwrap_or_default();
            }
            state
                .feed
                .weather_update(bridge.0.world(), &mut state.assets);
            if ahead && state.feed.world_rolls() {
                match roll_ahead(&bridge, state) {
                    Ok(()) => {}
                    // The preview keeps running, as with a frame it cannot
                    // build.
                    Err(e) if state.preview => warn!("preview: world rolls: {e}"),
                    Err(e) => return Err(e.into()),
                }
            }
            // ... and its cursor step, on the client seed after the weather:
            // the cursor steps whether or not the GPU is free here
            // (otherwise the seed would be one step short by the next
            // sound tick).
            cursor_step(
                ui.as_deref_mut(),
                &bridge,
                schedule.as_deref(),
                tick,
                &mut cursor_ticks.0,
                true,
            )?;
        }
        return Ok(());
    }
    if let Some(d) = drawn.as_deref_mut() {
        d.0 = tick;
    }
    let draws = ui_frame.as_ref().map_or(&[][..], |f| &f.draws[..]);
    state.feed.prepare(bridge.0.world(), &mut state.assets)?;
    // The drawn frame's pass over the client's kept light list, held by
    // the model between frames. A check run lights the frame with the
    // recorded light quality (it follows the host's wall clock and draw
    // rate; the schedule's `quality` column).
    let feed = &mut state.feed;
    feed.set_light_quality(
        schedule
            .as_ref()
            .and_then(|s| s.quality.get(&tick).copied())
            .or_else(|| {
                config
                    .as_ref()
                    .filter(|_| schedule.is_none())
                    .map(|c| c.settings.light_quality)
            }),
    );
    bridge
        .0
        .light_frame(|w, lights| feed.light_frame(w, lights));
    let placed = match ui_frame {
        Some(_) => placed,
        None => super::feed::camera_at(bridge.0.world(), state.feed.as_ref(), anchor)?,
    };
    // Client missiles read the drawn frame's unit origin (the camera's
    // world position) at the next client update.
    if let (true, Some((cam, _))) = (draw, placed.as_ref()) {
        bridge.0.set_unit_origin(cam);
    }
    // This pass's clicks may have put the local player in a new mode (a
    // click on the floor starts the walk, the town walk or the run at
    // once), after this pass's art load ran: the art of the modes the
    // frame draws is made resident now, so the tick draws the player
    // instead of skipping a unit whose COF is not loaded yet (the
    // character vanished for a tick on its first move).
    if let Some(art) = unit_art.as_deref() {
        for line in art.0.ensure(bridge.0.world(), &mut state.assets) {
            warn!("{line}");
        }
    }
    let built = build_frame_placed(
        bridge.0.world(),
        draws,
        state.rules.as_ref(),
        state.feed.as_mut(),
        &state.assets,
        placed,
    );
    // The draw-order fill's Y sort persists in the client's room lists,
    // on every frame the fill ran, the frames whose image is not built
    // included.
    for (room, order) in state.feed.take_unit_orders() {
        bridge.0.set_room_order(room, &order);
    }
    // The frame's cursor step draws on the client seed last, after the
    // frame's pass 4 / 9 rain draws (the weather view's seed commit, run
    // by the build). With a schedule, a drawn frame without a recorded clock is the
    // last one only (`FrameSchedule::parse`): its step follows the dump.
    cursor_step(
        ui.as_deref_mut(),
        &bridge,
        schedule.as_deref(),
        tick,
        &mut cursor_ticks.0,
        false,
    )?;
    let mut frame = match built {
        Ok(f) => f,
        // The preview keeps running; the frame is not presented.
        Err(e) if state.preview => {
            state.skipped_frames += 1;
            let message = e.to_string();
            if state.preview_error.as_deref() != Some(message.as_str()) {
                warn!("preview: frame not drawn: {message}");
                state.preview_error = Some(message);
            }
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    // Each unit draw of the frame takes
    // one from the unit's hit flash counter (the frame already holds the
    // pre-pass it drew).
    bridge.0.take_hit_flashes(&frame.unit_draws);
    *state.camera.write().unwrap_or_else(|e| e.into_inner()) = frame.camera;
    for m in state.ground_items.add_to_frame(
        bridge.0.world(),
        state.feed.as_ref(),
        &mut state.assets,
        &mut frame,
    ) {
        bevy::log::warn!(target: "d2_asset_fallback", "{m}");
    }
    for m in state.missiles.add_to_frame(
        bridge.0.world(),
        state.feed.as_ref(),
        &mut state.assets,
        &mut frame,
    ) {
        bevy::log::warn!(target: "d2_asset_fallback", "{m}");
    }
    // The level background of pass 1 (the first thing drawn).
    if let (Some(v), Some((_, mode)), Some(at)) = (state.background_view.as_mut(), placed, anchor) {
        for m in v.add_to_frame(
            bridge.0.world(),
            mode.get(),
            at.player.client().x,
            &mut state.assets,
            &mut frame,
        ) {
            warn!("preview: {m}");
        }
    }
    // The rooms of a whole preset level the client's level generator made
    // since the last frame (reported while the message that generated it
    // ran, before this frame's automap).
    if let Some(a) = state.automap.as_mut() {
        let world = bridge.0.world();
        // The act record's file opens first (S→C 0x03 ran before the
        // level generation it started), so the callbacks' layer switch
        // loads it.
        a.act(world)?;
        // The town levels' town art, likewise.
        a.drlg_towns(world)?;
        for (id, mut room) in state.feed.automap_rooms(world)? {
            a.preset_room(&mut room, &mut [])?;
            state
                .feed
                .automap_marked(id, &room, crate::ui::automap::place::REC_AUTOMAP);
        }
    }
    // The open automap's draw pass.
    if let (Some(a), Some(v)) = (state.automap.as_mut(), state.automap_view.as_mut()) {
        let world = bridge.0.world();
        if let (Some((_, mode)), Some(at)) = (placed, anchor) {
            let strings = ui.as_ref().map(|u| u.strings.as_ref());
            let fonts = ui
                .as_ref()
                .and_then(|u| u.original.as_ref())
                .and_then(|o| o.font_measure());
            let near = state.feed.near_rooms(world)?.map(|n| n.clone());
            let units = match near {
                Some(near) => {
                    let feed = &state.feed;
                    Some(super::automap_view::marker_units(
                        world,
                        &near,
                        &state.monster_labels,
                        strings,
                        at.player.client(),
                        &|u| feed.unit_position(u).ok().map(|p| p.client()),
                    ))
                }
                None => None,
            };
            for m in v.add_to_frame(
                a,
                world,
                mode,
                at.player.client(),
                units.as_deref(),
                Some(&super::automap_view::AutomapText {
                    rules: state.rules.as_ref(),
                    width: &|font, t| fonts.as_ref().and_then(|f| f.width_a(font, t)),
                    string: &|id| strings.and_then(|s| s.get_id(id)).map(<[u16]>::to_vec),
                    loader: ui.as_ref().and_then(|u| u.text.as_ref()),
                }),
                &mut state.assets,
                &mut frame,
            ) {
                warn!("preview: {m}");
            }
        }
    }
    // The automap reveal of this frame, after the draw marked its
    // records.
    if let Some(a) = state.automap.as_mut() {
        let world = bridge.0.world();
        state.feed.units_undrawn(&frame.draw_zero);
        let near = state.feed.near_rooms(world)?;
        a.frame(world, near)?;
    }
    let blank_screen = state.feed.blank_screen(bridge.0.world())?;
    let loads = bridge.0.world().act_loads;
    note_act_loads(&mut state.cycle, &mut state.act_loads, loads);
    if let Some(d) = dump.as_deref_mut().filter(|d| !d.done()) {
        d.seen += 1;
        let world = bridge.0.world();
        let open_mode = state.feed.open_mode(world).ok().map(|m| m.get());
        // Every requested tick this frame reaches is dumped from it. A tick
        // the recorded run drew more than once is the paused loop (the Esc menu
        // stops the client updates): its compared draw is the last one, with
        // the menu open, so D2RUST dumps once the menu is.
        let menu_open = ui
            .as_ref()
            .and_then(|u| u.original.as_ref())
            .is_some_and(|o| o.is_open(9));
        let paused_loop = schedule
            .as_ref()
            .and_then(|s| s.draws.get(&tick))
            .is_some_and(|v| v.len() > 1);
        if paused_loop && menu_open {
            d.menu_draws += 1;
        }
        while !d.done() && tick >= d.request.at_ticks[d.next] && (!paused_loop || d.menu_draws >= 2)
        {
            let dir = d.request.dir_for(d.next);
            d.next += 1;
            let frame_in = crate::facts::export::DumpFrame {
                world,
                frame: &frame,
                assets: &state.assets,
                cycle: &state.cycle,
                blank_screen,
                open_mode,
                seq: d.seen,
                inputs: crate::facts::export::FrameInputs {
                    player: anchor.map(|a| {
                        let c = a.player.client();
                        (c.x, c.y)
                    }),
                    light_quality: schedule
                        .as_ref()
                        .and_then(|s| s.quality.get(&tick).copied())
                        .or_else(|| {
                            config
                                .as_ref()
                                .filter(|_| schedule.is_none())
                                .map(|c| c.settings.light_quality)
                        }),
                    weather: state.feed.level_weather(),
                },
            };
            match crate::facts::export::dump(&d.request, &dir, &frame_in) {
                Ok(()) => {
                    println!(
                        "play: rendering facts of tick {tick} ({} items) written to {}",
                        frame.items.len(),
                        dir.display()
                    );
                    if d.done() {
                        exit.write(AppExit::Success);
                    }
                }
                Err(e) => {
                    eprintln!("play: --dump-draws failed: {e}");
                    d.next = d.request.at_ticks.len();
                    exit.write(AppExit::error());
                }
            }
        }
    }
    let use_gpu = gpu.is_some();
    let bridge_frame = bridge.0.world().frames;
    state.last_tags.clear();
    state.last_tags.extend(frame.items.iter().map(|i| i.tag));
    state.last_ui.clear();
    state.last_ui.extend_from_slice(draws);
    state
        .drawn_anim
        .observe(tick, bridge.0.world().tick_rate.hz(), &frame.unit_frames);
    state.last = Some(FrameStats {
        bridge_frame,
        server_tick: tick,
        items: frame.items.len(),
        units_drawn: frame.units_drawn,
        units_hidden: frame.units_hidden,
        ui_sent: ui_frame.as_ref().map_or(0, |f| f.sent),
        ui_unhandled: ui_frame.as_ref().map_or(0, |f| f.unhandled.len()),
        gpu: use_gpu,
    });
    let image = match &target {
        Some(t) => t.image.clone(),
        None => {
            // The GPU path writes the texture itself: a target texture
            // (zeros) that the main world never rewrites.
            let mut image = if use_gpu {
                {
                    let mut image = Image::new_target_texture(
                        play_view().width,
                        play_view().height,
                        TextureFormat::Rgba8UnormSrgb,
                        None,
                    );
                    // Copy source too: tests read the presented frame back.
                    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
                    image
                }
            } else {
                rgba_image(vec![
                    0;
                    (play_view().width * play_view().height * 4) as usize
                ])
            };
            image.sampler = ImageSampler::nearest();
            let image = images.add(image);
            let layer = RenderLayers::layer(PRESENT_LAYER);
            commands.spawn((
                Camera2d,
                Camera {
                    order: 1,
                    clear_color: ClearColorConfig::Custom(Color::srgb_u8(0, 0, 0)),
                    ..default()
                },
                Msaa::Off,
                Tonemapping::None,
                layer.clone(),
                crate::app::session::GameEntity,
            ));
            let sprite = commands
                .spawn((
                    Sprite::from_image(image.clone()),
                    WorldViewSprite,
                    layer,
                    crate::app::session::GameEntity,
                ))
                .id();
            commands.insert_resource(WorldViewTarget {
                image: image.clone(),
                sprite,
            });
            image
        }
    };
    match gpu {
        Some(mut g) => {
            let g = &mut *g;
            g.atlas.ensure_for(&state.assets.frames, &frame.items)?;
            if g.atlas.version() != g.held {
                g.held = g.atlas.version();
                g.pages = Arc::new(g.atlas.share_pages(&g.pages));
            }
            let (mut packed, plan) =
                g.atlas
                    .pack_cycle(&state.cycle, blank_screen, &frame, &state.assets)?;
            // The UI and the local player stay fixed on screen while the
            // camera blends between ticks (`super::smooth`).
            let local = bridge.0.world().local_player.map(|k| k.guid);
            packed.set_layers(|i| super::native_ui::item_layer(&frame.items[i], local));
            // The UI at the output resolution when the frame is shown
            // enlarged (`super::native_ui`).
            let presentation = windows
                .single()
                .ok()
                .and_then(|w| crate::ui::edge::presentation(w).ok());
            let ui_job = match presentation.as_ref().and_then(|p| {
                let place = super::native_ui::placement(p)?;
                let targets = super::native_ui::ensure_targets(
                    &mut commands,
                    &mut images,
                    native.as_deref(),
                    p,
                )?;
                Some((place, targets))
            }) {
                Some((place, targets)) => {
                    packed.params.native_ui = 1;
                    Some(super::native_ui::ui_job(
                        &frame.items,
                        &packed,
                        &state.assets,
                        g.atlas.slots(),
                        place,
                        &mut g.ui_tables,
                        super::ui_art::current(),
                        &targets,
                    )?)
                }
                None => None,
            };
            g.seq += 1;
            g.pending = Some((g.seq, plan));
            let packed = Arc::new(packed);
            // Kept for the display frames until the next tick: its units
            // move between ticks (`super::unit_motion`).
            commands.insert_resource(super::unit_motion::TickFrame {
                tick,
                seq: g.seq,
                pages_version: g.held,
                seen: super::unit_motion::seen_of(
                    bridge.0.world(),
                    state.feed.as_ref(),
                    &frame.movers,
                ),
                movers: std::mem::take(&mut frame.movers),
                items: Arc::new(std::mem::take(&mut frame.items)),
                packed: packed.clone(),
            });
            commands.insert_resource(ComposeJob {
                seq: g.seq,
                frame: bridge_frame,
                packed,
                pages: g.pages.clone(),
                pages_version: g.held,
                palette: Arc::new(state.assets.palette.clone()),
                base_after,
                sync_readback: !chain,
                target: image,
                layers: layers.as_ref().map(|l| l.targets()),
                ui: ui_job,
            });
        }
        None => {
            let rgba = compose_cycle_cpu(&mut state.cycle, blank_screen, &frame, &state.assets)?;
            let mut image = images
                .get_mut(&image)
                .ok_or("world view image asset is gone")?;
            image.data = Some(rgba);
        }
    }
    Ok(())
}

/// Places a frame quad (or sprite) where `p` puts the frame: its scale on
/// each axis and the image's top-left on the presentation's (left, top),
/// where the cursor mapping expects it
/// ([`crate::ui::Presentation::centre_offset`]). `p` is in physical pixels;
/// Bevy places things in logical units, so both the scale and the offset
/// are divided by the window's `scale_factor`.
pub fn place_frame(p: &crate::ui::Presentation, scale_factor: f32, t: &mut Transform) {
    let (dx, dy) = p.centre_offset();
    t.scale = Vec3::new(p.sx / scale_factor, p.sy / scale_factor, 1.0);
    t.translation.x = dx / scale_factor;
    t.translation.y = dy / scale_factor;
}

/// Hands the sharp material the presentation's scale, which sets how wide
/// the blended edge between two game pixels is. Only on a change: touching
/// the material re-uploads it.
pub fn scale_sharp(
    p: &crate::ui::Presentation,
    sharp: Option<&MeshMaterial2d<SharpMaterial>>,
    materials: Option<&mut Assets<SharpMaterial>>,
) {
    let (Some(sharp), Some(materials)) = (sharp, materials) else {
        return;
    };
    let want = super::sharp::scale(p.sx, p.sy);
    if materials.get(&sharp.0).is_some_and(|m| m.scale != want) {
        if let Some(mut m) = materials.get_mut(&sharp.0) {
            m.scale = want;
        }
    }
}

/// Scales the game frame to the window ([`edge::presentation`], the same
/// placement the cursor mapping uses), through the sharp material when the
/// app renders.
fn present_scale(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut sprites: Query<
        (&mut Transform, Option<&MeshMaterial2d<SharpMaterial>>),
        With<WorldViewSprite>,
    >,
    mut materials: Option<ResMut<Assets<SharpMaterial>>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(p) = edge::presentation(window) else {
        return;
    };
    for (mut t, sharp) in &mut sprites {
        place_frame(&p, window.scale_factor(), &mut t);
        scale_sharp(&p, sharp, materials.as_deref_mut());
    }
}

/// Turns a frame sprite into a quad drawn with the sharp material, once per
/// new frame image. The quad is the frame image's size in game pixels (from
/// the image itself, whatever the frame's size), so [`place_frame`] scales
/// it exactly as it would the sprite.
pub fn sharp_quad(
    commands: &mut Commands,
    e: Entity,
    frame: Handle<Image>,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<SharpMaterial>,
) {
    let Some(mut image) = images.get_mut(&frame) else {
        return;
    };
    image.sampler = super::sharp::frame_sampler();
    let size = image.size_f32();
    let quad = meshes.add(Rectangle::new(size.x, size.y));
    let material = materials.add(SharpMaterial::new(frame));
    commands
        .entity(e)
        .remove::<Sprite>()
        .insert((Mesh2d(quad), MeshMaterial2d(material)));
}

/// The game frame's sprite becomes a sharp quad ([`sharp_quad`]).
fn present_sharp(
    mut commands: Commands,
    sprites: Query<(Entity, &Sprite), With<WorldViewSprite>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SharpMaterial>>,
) {
    for (e, sprite) in &sprites {
        sharp_quad(
            &mut commands,
            e,
            sprite.image.clone(),
            &mut images,
            &mut meshes,
            &mut materials,
        );
    }
}

#[cfg(test)]
mod act_load_tests {
    use super::*;
    use crate::scene::{FrameImage, MapTable};

    #[test]
    fn the_frame_after_an_act_load_presents_all_index_0() {
        // Framebuffer all 5, BlankScreen 1, nothing
        // drawn, 800 × 600, counter 1 → all 0, counter back to 0.
        let mut c = FrameCycle::with_pixels(800, 600, vec![5; 800 * 600]).unwrap();
        let mut seen = 0;
        note_act_loads(&mut c, &mut seen, 0);
        assert_eq!(c.post_clear(), 0, "no 0x03 yet");
        note_act_loads(&mut c, &mut seen, 1);
        assert_eq!(c.post_clear(), 1);
        let none: Vec<FrameImage> = Vec::new();
        let out = c.compose(true, &[], &none, &MapTable::default()).unwrap();
        assert!(out.iter().all(|&p| p == 0));
        assert_eq!(c.post_clear(), 0);
        // The same count again: no clear; the next frame keeps rows
        // 553–599 (BlankScreen clears rows 0–552 only).
        note_act_loads(&mut c, &mut seen, 1);
        assert_eq!(c.post_clear(), 0);
        // Two loads before one frame: one cleared frame.
        note_act_loads(&mut c, &mut seen, 3);
        assert_eq!(c.post_clear(), 1);
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Order(Vec<&'static str>);

    fn outputs(mut o: ResMut<Order>) {
        o.0.push("outputs");
    }
    fn player_walk(mut o: ResMut<Order>) {
        o.0.push("player walk");
    }
    fn monster_walk(mut o: ResMut<Order>) {
        o.0.push("monster walk");
    }

    /// The three `PreUpdate` systems that take the bridge after the bridge frame run
    /// in a pinned order, whatever their registration order: the frame's
    /// outputs, the player walk, the monster tracks.
    #[test]
    fn the_bridge_systems_after_the_frame_run_in_a_pinned_order() {
        for _ in 0..8 {
            let mut app = App::new();
            app.init_resource::<Order>();
            app.add_systems(PreUpdate, monster_walk.in_set(PreviewOrder::MonsterWalk));
            app.add_systems(PreUpdate, player_walk.in_set(PreviewOrder::PlayerWalk));
            app.add_systems(PreUpdate, outputs.in_set(PreviewOrder::Outputs));
            configure_preview_order(&mut app);
            app.update();
            assert_eq!(
                app.world().resource::<Order>().0,
                ["outputs", "player walk", "monster walk"]
            );
        }
    }
}

#[cfg(test)]
mod click_sound_tests {
    use super::*;

    #[test]
    fn a_world_clicks_sounds_become_player_event_requests_in_order() {
        let p = UnitKey::new(0, 1);
        let outs = [
            ClickOut::Sound(0x13),
            ClickOut::Retarget,
            ClickOut::Sound(0x1F),
        ];
        assert_eq!(
            click_sounds(Some(p), &outs),
            vec![
                SoundRequest::PlayerEvent {
                    unit: p,
                    event: 0x13
                },
                SoundRequest::PlayerEvent {
                    unit: p,
                    event: 0x1F
                },
            ]
        );
        assert!(click_sounds(None, &outs).is_empty());
    }
}

#[cfg(test)]
mod schedule_tests {
    use super::FrameSchedule;

    #[test]
    fn the_ready_column_parses_by_class_and_mode() {
        let text = "# frame-schedule 1\n# tick_rate 60\n# cursor_last 0\n# cursor_idle 40\n\
                    tick\tnow\tquality\tcursor_x\tcursor_y\tcursor_idle\tready\n\
                    2\t40\t2\t320\t240\t40\t147:6,152:901f\n\
                    3\t80\t2\t320\t240\t40\tnone\n\
                    4\t120\t2\t320\t240\t40\t-\n";
        let s = FrameSchedule::parse(text).unwrap();
        let r = &s.ready[&2];
        assert!(r.ready(147, 1) && r.ready(147, 2) && !r.ready(147, 0));
        assert!(r.ready(152, 15) && r.ready(152, 12) && !r.ready(152, 5));
        assert!(!r.ready(19, 1));
        assert!(s.ready[&3].masks.is_empty());
        assert!(!s.ready.contains_key(&4));
        let bad = text.replace("147:6,", "147:zz,");
        assert!(FrameSchedule::parse(&bad).is_err());
    }

    #[test]
    fn a_tick_drawn_twice_keeps_both_clocks() {
        let text = "# frame-schedule 1\n# tick_rate 60\n# cursor_last 0\n# cursor_idle 10\n\
                    tick\tnow\n22\t100\n22\t120\n23\t150\n24\t-\n";
        let s = FrameSchedule::parse(text).unwrap();
        assert_eq!(s.first_now(22), Some(100));
        assert_eq!(s.extra_draws(22), &[Some(120)][..]);
        assert_eq!((s.first_now(23), s.extra_draws(23).len()), (Some(150), 0));
        // a tick drawn once keeps the old reading
        assert_eq!(s.now(22), Some(120));
    }

    #[test]
    fn a_recorded_schedule_parses_and_a_missing_clock_fails_loudly() {
        let text = "# frame-schedule 1\n# tick_rate 60\n# cursor_last 0\n# cursor_idle 6402500\n\
                    tick\tnow\n3\t6403156\n5\t6403281\n6\t-\n";
        let s = FrameSchedule::parse(text).unwrap();
        assert_eq!((s.cursor_last, s.cursor_idle), (0, 6_402_500));
        assert!(s.drawn(3) && !s.drawn(4) && s.drawn(5) && s.drawn(6));
        assert_eq!(
            (s.now(3), s.now(5), s.now(6)),
            (Some(6_403_156), Some(6_403_281), None)
        );
        // Only the last frame may lack its clock; no version, no timers:
        // refused, never a fallback clock.
        let gap = text.replace("3\t6403156", "3\t-");
        assert!(FrameSchedule::parse(&gap).is_err());
        // The optional light-quality column (a recorded host input).
        let q = FrameSchedule::parse(
            &text
                .replace("tick\tnow\n", "tick\tnow\tquality\n")
                .replace("3\t6403156", "3\t6403156\t2")
                .replace("5\t6403281", "5\t6403281\t0"),
        )
        .unwrap();
        assert_eq!(
            (q.quality.get(&3), q.quality.get(&5), q.quality.get(&6)),
            (Some(&2), Some(&0), None)
        );
        assert!(
            FrameSchedule::parse(&text.replace("# frame-schedule 1\n# tick_rate 60\n", ""))
                .is_err()
        );
        assert!(FrameSchedule::parse(&text.replace("# cursor_idle 6402500\n", "")).is_err());
    }

    #[test]
    fn script_steps_reach_the_pass_after_the_draw_of_their_tick() {
        let text = "# frame-schedule 1\n# tick_rate 60\n# cursor_last 0\n# cursor_idle 0\n\
                    tick\tnow\n39\t1\n41\t2\n";
        let s = FrameSchedule::parse(text).unwrap();
        // `frame 40` waits for tick 39: drawn at 39, so the steps come in
        // the pass of tick 40 and the draw of 41 is the first to see them
        assert_eq!(super::script_tick(39, Some(&s)), 38);
        assert_eq!(super::script_tick(40, Some(&s)), 40);
        assert_eq!(super::script_tick(41, Some(&s)), 40);
        assert_eq!(super::script_tick(39, None), 39);
    }

    #[test]
    fn a_drawn_tick_before_an_undrawn_one_gets_an_input_only_pass_for_due_steps() {
        use super::super::input_script::{parse, InputScript};
        let text = "# frame-schedule 1\n# tick_rate 60\n# cursor_last 0\n# cursor_idle 0\n\
                    tick\tnow\n39\t1\n41\t2\n";
        let s = FrameSchedule::parse(text).unwrap();
        let script = InputScript::new(parse("frame 40; click 600 300").unwrap());
        // 39 drawn, 40 not, `frame 40` due at 39: an input-only pass
        assert!(super::input_only_after(39, Some(&s), &script));
        // 41 drawn and 42 not, but nothing due: no pass
        let later = InputScript::new(parse("frame 50; click 600 300").unwrap());
        assert!(!super::input_only_after(41, Some(&s), &later));
        // undrawn tick, or no schedule: the step runs in its own pass
        assert!(!super::input_only_after(40, Some(&s), &script));
        assert!(!super::input_only_after(39, None, &script));
        // consecutive drawn ticks: still a pass at 39
        let t2 = "# frame-schedule 1\n# tick_rate 60\n# cursor_last 0\n# cursor_idle 0\n\
                  tick\tnow\n39\t1\n40\t2\n";
        let s2 = FrameSchedule::parse(t2).unwrap();
        assert!(super::input_only_after(39, Some(&s2), &script));
    }
}

#[cfg(test)]
mod cursor_jump_tests {
    use super::cursor_jump_move;
    use crate::ui::Point;

    #[test]
    fn the_jump_move_misses_the_top_border() {
        assert_eq!(cursor_jump_move(Point::new(590, 10)), Point::new(590, 7));
        assert_eq!(cursor_jump_move(Point::new(120, 240)), Point::new(120, 237));
    }
}
