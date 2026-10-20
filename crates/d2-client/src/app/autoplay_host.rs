//! `d2-client autoplay-host`: the `play` client, headless, driven over a
//! line protocol on stdin / stdout, for the autoplay bot
//! (`tools/autoplay/`). The app is the one `d2-client play` runs
//! ([`super::play::add_live_client`] on Bevy's `MinimalPlugins`, as the
//! `play_smoke` test runs it): the server thread, the bridge, the
//! game's UI and the world-click path. Only the window is missing; the
//! bot's input goes into the UI queue exactly where the window's pointer
//! and key events go (`world_view::present::ui_input` /
//! `script_input`), so a click passes the panels, the hover pick and the
//! world-click dispatcher like a player's.
//!
//! Commands use pointer/key events, client interaction requests and reads;
//! they never mutate the server state. The reads (`state`,
//! `query`, `map`) take the server snapshot of `state-dump` (`d2_sim::debug::state`)
//! and the DRLG's rooms on the server thread, reading only, and
//! the client's UI and model.
//! Commands and reads use the shared `autoplay-1` protocol
//! ([`super::control::protocol`]), also used by `play --control`.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use bevy::prelude::*;
use serde_json::{json, Value};

use super::autoplay_front::{FrontPhase, FrontStep};
use super::control::protocol;
pub use super::control::protocol::{parse_command, Command, FORMAT};
use super::play::{add_live_client, LiveClient};
use super::play_start::{self, CliStart};
use super::server_thread::ThreadLink;
use super::single_player::{self, GameData, Link};
use super::state_dump::{step_ms, StepClock, START_MS};
use crate::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use crate::bridge::BridgeResource;
use crate::controls::Preset;
use crate::ui::{Point, PointerButton, UiEvent};
use crate::world_view::input_script::{self, UnitSel};
use crate::world_view::WorldViewUi;

/// The server thread, shared between the app's link and the reads.
pub type Server = Arc<Mutex<ThreadLink<Link<StepClock>>>>;

/// Message ids with counts (diagnostics).
type SentLog = protocol::MessageLog;

/// The app's link: the shared server thread, counting the C→S ids.
struct SharedLink(Server, SentLog, SentLog);

impl ServerLink for SharedLink {
    fn protocol_version(&self) -> u32 {
        self.0.lock().expect("server lock").protocol_version()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.0.lock().expect("server lock").tick_rate()
    }
    fn server_frame(&mut self) -> Option<i32> {
        self.0.lock().expect("server lock").server_frame()
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        if let Some(&id) = msg.first() {
            *self.1.lock().expect("sent lock").entry(id).or_default() += 1;
        }
        self.0.lock().expect("server lock").send(queue, msg)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.0.lock().expect("server lock").pump()
    }
    fn set_display_catch_up(&mut self, enabled: bool) {
        self.0
            .lock()
            .expect("server lock")
            .set_display_catch_up(enabled)
    }
    fn tick_pending(&mut self) -> bool {
        self.0.lock().expect("server lock").tick_pending()
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        let msgs = self.0.lock().expect("server lock").receive();
        let mut log = self.2.lock().expect("received lock");
        for id in msgs.iter().filter_map(|m| m.first()) {
            *log.entry(*id).or_default() += 1;
        }
        msgs
    }
    fn server_paths(&mut self) -> Vec<crate::bridge::critter_path::ServerPath> {
        super::save::server_paths_on(&mut self.0.lock().expect("server lock"))
    }
}

/// `autoplay-host`'s options.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostArgs {
    /// `--save FILE.d2s`; none: `--new`'s character.
    pub save: Option<PathBuf>,
    /// `--new CLASS NAME`.
    pub new: Option<(String, String)>,
    pub seed: Option<u32>,
    pub difficulty: u8,
    pub game_dir: Option<PathBuf>,
    /// `--res WxH`: the play frame (default 800 × 600), as `play --res`.
    pub res: Option<crate::rules::camera::FrameSize>,
    /// `--front`: start on the front end's main menu
    /// ([`super::autoplay_front`]); the menus pick the character.
    pub front: bool,
    /// `--save-dir DIR`: the Save folder the front end lists and creates
    /// characters in (`--front`).
    pub save_dir: Option<PathBuf>,
}

/// Parses the options after `autoplay-host`.
pub fn parse_args(args: &[String]) -> Result<HostArgs> {
    let mut a = HostArgs::default();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--save" => a.save = Some(PathBuf::from(value()?)),
            "--new" => {
                let class = value()?.clone();
                let name = value()?.clone();
                a.new = Some((class, name));
            }
            "--seed" => a.seed = Some(value()?.parse().context("--seed")?),
            "--difficulty" => {
                let v = value()?;
                a.difficulty = single_player::parse_difficulty(v)
                    .with_context(|| format!("--difficulty {v}"))?;
            }
            "--game-dir" => a.game_dir = Some(PathBuf::from(value()?)),
            "--front" => a.front = true,
            "--save-dir" => a.save_dir = Some(PathBuf::from(value()?)),
            "--res" => {
                let v = value()?;
                a.res = Some(
                    crate::rules::camera::FrameSize::parse(v)
                        .with_context(|| format!("--res {v}: use 800x600, 640x480 or 1068x600"))?,
                );
            }
            other => bail!("autoplay-host: unknown option {other}"),
        }
    }
    if a.front {
        if a.save.is_some() || a.new.is_some() {
            bail!("autoplay-host --front picks the character in the menus: no --save or --new");
        }
        if a.save_dir.is_none() {
            bail!("autoplay-host --front needs --save-dir DIR");
        }
    } else if a.save.is_some() == a.new.is_some() {
        bail!("autoplay-host needs one of --save FILE, --new CLASS NAME or --front");
    }
    Ok(a)
}

/// Selects the nearest matching client-model unit without a frame boundary.
/// The ordering is the unit-click ordering: distance to the camera centre,
/// then unit key. The normal client sender leaves range checks to the server.
fn select_interact_unit(
    world: &crate::bridge::world::ClientWorld,
    cam: &crate::rules::camera::Camera,
    sel: &UnitSel,
) -> Result<crate::bridge::world::UnitKey, String> {
    let size = crate::rules::camera::FrameSize::play();
    world
        .units
        .iter()
        .filter_map(|(key, unit)| {
            if key.unit_type != sel.unit_type
                || (!sel.any() && !sel.classes[..usize::from(sel.count)].contains(&unit.class))
                || (sel.any() && matches!(unit.mode, 0 | 12))
            {
                return None;
            }
            let p = crate::bridge::hover::unit_feet(cam, key.unit_type, unit.position?);
            let (dx, dy) = (
                i64::from(p.0 - size.width / 2),
                i64::from(p.1 - size.height / 2),
            );
            Some((dx * dx + dy * dy, *key))
        })
        .min()
        .map(|(_, key)| key)
        .ok_or_else(|| "no such unit".into())
}

fn snapshot_approach(
    units: &Value,
    cam: &crate::rules::camera::Camera,
    sel: &UnitSel,
) -> Result<(crate::bridge::world::UnitKey, (u16, u16))> {
    let size = crate::rules::camera::FrameSize::play();
    let (cx, cy) = (size.width / 2, size.height / 2);
    let (_, guid, cell) = units
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|u| {
            if u["ut"].as_u64()? != u64::from(sel.unit_type)
                || (!sel.any()
                    && !sel.classes[..usize::from(sel.count)].contains(&(u["cl"].as_u64()? as u32)))
                || (sel.any() && matches!(u["m"].as_u64()?, 0 | 12))
            {
                return None;
            }
            let cell = (u["x"].as_u64()? as u16, u["y"].as_u64()? as u16);
            let (x, y) = crate::bridge::hover::unit_feet(cam, sel.unit_type, cell);
            let (dx, dy) = (i64::from(x - cx), i64::from(y - cy));
            Some((dx * dx + dy * dy, u["g"].as_u64()?, cell))
        })
        .min()
        .context("interactunit: no such unit in client or host snapshot")?;
    Ok((
        crate::bridge::world::UnitKey::new(sel.unit_type, guid as u32),
        cell,
    ))
}

/// Project a ground click, removing the unit drawer's extra eight rows.
/// Using drawn feet unchanged makes the click inverse select (x + 1, y + 1).
fn walk_point(cam: &crate::rules::camera::Camera, cell: (u16, u16)) -> Point {
    let (x, y) = crate::bridge::hover::unit_feet(cam, 1, cell);
    Point { x, y: y - 8 }
}

/// Find a short walking waypoint using only the host's read-only map.
/// Local clicks let the normal player pathfinder handle footprint details.
fn approach_waypoint(
    map: &Value,
    from: (u16, u16),
    target: (u16, u16),
    reach: u16,
) -> Result<(u16, u16)> {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    type Cell = (u16, u16);
    let distance = |a: Cell, b: Cell| a.0.abs_diff(b.0).max(a.1.abs_diff(b.1));
    let mut free = BTreeSet::new();
    for level in map["levels"].as_array().into_iter().flatten() {
        for room in level["rooms"].as_array().into_iter().flatten() {
            let grid = &room["grid"];
            // Unstreamed rooms have no collision grid yet. Their bounds
            // guide an exploratory click; the server still checks movement,
            // and the next pass replans against newly streamed collision.
            let rect = grid["rect"].as_array().or_else(|| room["rect"].as_array());
            let Some(rect) = rect else { continue };
            let (x, y, w) = (
                rect[0].as_u64().unwrap_or(0) as u16,
                rect[1].as_u64().unwrap_or(0) as u16,
                rect[2].as_u64().unwrap_or(0) as usize,
            );
            if w == 0 {
                continue;
            }
            let unknown;
            let cells = if let Some(cells) = grid["cells"].as_str() {
                cells
            } else {
                unknown = ".".repeat(w * rect[3].as_u64().unwrap_or(0) as usize);
                &unknown
            };
            for (i, cell) in cells.bytes().enumerate() {
                let p = (x + (i % w) as u16, y + (i / w) as u16);
                if cell != b'#' {
                    free.insert(p);
                }
            }
        }
    }
    // The server clears the moving player's NO_PATH centre marker before
    // testing the next footprint. The map folds that marker into `#`,
    // but the player's current cell must remain usable by adjacent steps.
    free.insert(from);
    // Players use size 2 / pattern 1: centre and its four cardinal
    // neighbours (path::collision::plus_value). A free centre beside a
    // wall is not a valid walking destination for that footprint.
    let walkable: BTreeSet<Cell> = free
        .iter()
        .copied()
        .filter(|&(x, y)| {
            x > 0
                && y > 0
                && x < u16::MAX
                && y < u16::MAX
                && [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
                    .iter()
                    .all(|p| free.contains(p))
        })
        .collect();
    let mut queue = VecDeque::from([from]);
    let mut previous = BTreeMap::from([(from, from)]);
    while let Some(p) = queue.pop_front() {
        if distance(p, target) <= reach {
            let mut path = vec![p];
            while *path.last().unwrap() != from {
                path.push(previous[path.last().unwrap()]);
            }
            path.reverse();
            return Ok(path[path.len().saturating_sub(1).min(6)]);
        }
        if previous.len() > 100_000 {
            break;
        }
        for (dx, dy) in [
            (0, 1),
            (1, 0),
            (0, -1),
            (-1, 0),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ] {
            let (x, y) = (i32::from(p.0) + dx, i32::from(p.1) + dy);
            if !(0..=u16::MAX as i32).contains(&x) || !(0..=u16::MAX as i32).contains(&y) {
                continue;
            }
            let n = (x as u16, y as u16);
            if !walkable.contains(&n)
                || previous.contains_key(&n)
                || (dx != 0
                    && dy != 0
                    && (!walkable.contains(&(n.0, p.1)) || !walkable.contains(&(p.0, n.1))))
            {
                continue;
            }
            previous.insert(n, p);
            queue.push_back(n);
        }
    }
    bail!("interactunit: no walkable path from {from:?} to {target:?}")
}

/// The running host: the app, its server thread and its clock.
pub struct Host {
    app: App,
    server: Server,
    ms: Arc<AtomicU32>,
    passes: u64,
    sent: SentLog,
    /// The S→C message ids the client received, with counts.
    received: SentLog,
    /// Keys pressed since the last pass: held down for the next pass in
    /// the raw key state the window's input plugin keeps (the death and
    /// hardcore screens read it), released after it.
    keys: Vec<KeyCode>,
    /// The next pass runs with a primary window, so the window's key
    /// system (`ui_input`) reads `keys` itself (`rawkey`).
    window: bool,
}

impl Host {
    /// The play app on the user's files, joined (the local player exists).
    pub fn start(args: &HostArgs) -> Result<Self> {
        let dir = args
            .game_dir
            .clone()
            .or_else(|| std::env::var_os("D2_GAME_DIR").map(PathBuf::from));
        let data = GameData::select(dir.as_deref())?;
        let start = play_start::resolve(
            &CliStart {
                save: args.save.clone(),
                new: args.new.clone(),
                save_dir: None,
                difficulty: args.difficulty,
                hardcore: false,
            },
            &data,
            dir.as_deref(),
            None,
            None,
        )?;
        Self::start_with(
            data,
            start.character,
            start.start_flags,
            start.hardcore,
            args.seed,
        )
    }

    /// The play app on `data` with `character` (a save, or a character
    /// built by a runner such as `repro`), joined. `seed`: the game seed,
    /// `None` for the character's own.
    pub fn start_with(
        data: GameData,
        character: single_player::Character,
        start_flags: Option<u32>,
        hardcore: bool,
        seed: Option<u32>,
    ) -> Result<Self> {
        Self::start_configured(data, character, start_flags, hardcore, seed, false)
    }

    /// Diagnostic runners opt in to coverage; D2_COVERAGE=0 supports parity probes.
    pub fn start_with_coverage(
        data: GameData,
        character: single_player::Character,
        start_flags: Option<u32>,
        hardcore: bool,
        seed: Option<u32>,
    ) -> Result<Self> {
        let enabled = !std::env::var_os("D2_COVERAGE").is_some_and(|v| v == "0");
        Self::start_configured(data, character, start_flags, hardcore, seed, enabled)
    }

    fn start_configured(
        data: GameData,
        character: single_player::Character,
        start_flags: Option<u32>,
        hardcore: bool,
        seed: Option<u32>,
        enabled: bool,
    ) -> Result<Self> {
        let seed = single_player::game_seed(&character, seed);
        let hardcore = hardcore
            || super::save::base_save(&character).header.status & d2_formats::d2s::status::HARDCORE
                != 0;
        let GameData::Live(live) = data.clone();
        let ms = Arc::new(AtomicU32::new(START_MS));
        let speeds = single_player::walk_speeds(&data, &character)?;
        let (mut link, started) =
            single_player::start_with(data, seed, character.clone(), StepClock(ms.clone()))?;
        if hardcore {
            link.with(|l| l.host_mut().game.events.action.hooks().x.hardcore = true)?;
        }
        link.with(move |link| {
            link.host_mut().game.game.coverage.enabled = enabled;
            link.host_mut().coverage.enabled = enabled;
        })?;
        let server: Server = Arc::new(Mutex::new(link));
        let sent = SentLog::default();
        let received = SentLog::default();
        let mut app = App::new();
        app.insert_resource(crate::bridge::mirror::ScriptedClock(ms.clone()));
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>();
        add_live_client(
            &mut app,
            Box::new(SharedLink(server.clone(), sent.clone(), received.clone())),
            LiveClient {
                data: &live,
                request: &character,
                start_flags,
                prices: started.prices,
                speeds,
                hardcore,
                automap_files: None,
                gpu: false,
            },
        )?;
        app.world_mut()
            .non_send_mut::<WorldViewUi>()
            .root
            .coverage
            .enabled = enabled;
        // The default preset, not the user's saved controls: the bot's
        // keys mean the same on every machine.
        {
            let mut ui = app.world_mut().non_send_mut::<WorldViewUi>();
            let b = Preset::Original.bindings();
            if let (Some(o), Some(b)) = (ui.original.as_mut(), b.as_ref()) {
                o.set_belt_keys(b);
            }
            ui.bindings = b;
        }
        eprintln!("autoplay-host: seed {seed}");
        let mut host = Host {
            app,
            server,
            ms,
            passes: 0,
            sent,
            received,
            keys: Vec::new(),
            window: false,
        };
        for _ in 0..2000 {
            if host.local_player().is_some() {
                break;
            }
            host.step(1);
        }
        if host.local_player().is_none() {
            bail!("the join did not run in 2000 passes");
        }
        Ok(host)
    }

    /// The server thread (for runners that set up or read the game
    /// between passes).
    pub fn server(&self) -> Server {
        self.server.clone()
    }

    /// The app (for runners that add resources, such as an input script).
    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    /// The C→S message ids the client sent so far, with counts.
    pub fn sent_counts(&self) -> std::collections::BTreeMap<u8, u32> {
        self.sent.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Client passes run so far.
    pub fn passes(&self) -> u64 {
        self.passes
    }

    /// The server's frame counter now.
    pub fn server_frame(&self) -> Result<i32> {
        self.server
            .lock()
            .expect("server lock")
            .with(|l| l.host().game.game.frame)
            .map_err(|e| anyhow::anyhow!("{e:?}"))
    }

    /// The GUID of the local player, once it exists on the server.
    pub fn local_player(&self) -> Option<u32> {
        protocol::local_player(&self.server)
    }

    /// N client passes, the clock one game tick after each (one server tick).
    pub fn step(&mut self, n: u32) {
        for _ in 0..n {
            let keys = std::mem::take(&mut self.keys);
            {
                let mut input = self.app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                input.clear();
                for &k in &keys {
                    input.press(k);
                }
            }
            let window = std::mem::take(&mut self.window).then(|| {
                self.app
                    .world_mut()
                    .spawn((bevy::window::Window::default(), bevy::window::PrimaryWindow))
                    .id()
            });
            self.app.update();
            if let Some(w) = window {
                self.app.world_mut().despawn(w);
            }
            {
                let mut input = self.app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                for &k in &keys {
                    input.release(k);
                }
            }
            self.ms.store(
                step_ms(d2_sim::tick::TickRate::GAME, (self.passes + 1) as u32),
                Ordering::SeqCst,
            );
            self.passes += 1;
        }
    }

    /// The clickable elements of the open panels, in frame space, as the
    /// last pass left them ([`crate::world_view::ui_bind::ui_elements`]).
    fn ui_elements(&mut self) -> Vec<crate::ui::ui_ids::UiElement> {
        let world = self.app.world();
        let bridge = world.resource::<BridgeResource>();
        let ui = world.non_send::<WorldViewUi>();
        crate::world_view::ui_bind::ui_elements(&ui.root, bridge.0.world(), &*ui.strings)
    }

    fn ui(&mut self) -> Mut<'_, WorldViewUi> {
        self.app.world_mut().non_send_mut::<WorldViewUi>()
    }

    /// `state`: the server snapshot and the client's view.
    pub fn state(&mut self) -> Result<Value> {
        let counts = protocol::Counts {
            passes: self.passes,
            sent: &self.sent,
            received: &self.received,
        };
        protocol::state(self.app.world(), &self.server, &counts)
    }

    fn pointer(&mut self, at: Point, event: Option<UiEvent>) {
        protocol::pointer(&mut self.ui(), at, event);
    }

    fn key(&mut self, vk: u32) {
        let codes = protocol::key(&mut self.ui(), vk);
        self.keys.extend(codes);
    }

    fn map(&mut self) -> Result<Value> {
        protocol::map(&self.server)
    }

    /// Runs one command; its JSON reply line, `None` for `quit`.
    pub fn run(&mut self, c: Command) -> Result<Option<Value>> {
        let ok = |what: &str| Ok(Some(json!({"k": "ok", "cmd": what})));
        match c {
            Command::Step(n) => {
                self.step(n);
                Ok(Some(
                    json!({"k": "ok", "cmd": "step", "passes": self.passes}),
                ))
            }
            Command::WalkLevel(level) => {
                self.key(0x20);
                self.step(4);
                for _ in 0..64 {
                    let state = self.state()?;
                    let player = state["snap"]["units"]
                        .as_array()
                        .context("snapshot units")?
                        .iter()
                        .find(|u| u["ut"] == 0 && u["g"] == state["local"])
                        .context("walklevel: no local player")?;
                    if player["lv"] == level {
                        return ok("walklevel");
                    }
                    let current = player["lv"].as_u64().context("player level")?;
                    let from = (
                        player["x"].as_u64().context("player x")? as u16,
                        player["y"].as_u64().context("player y")? as u16,
                    );
                    let mut map = self.map()?;
                    let levels = map["levels"].as_array_mut().context("map levels")?;
                    levels.retain(|l| l["id"] == current || l["id"] == level);
                    let destination = levels
                        .iter_mut()
                        .find(|l| l["id"] == level)
                        .context("walklevel: target level is not on the current act's map")?;
                    let rect = destination["rect"].as_array().context("level bounds")?;
                    let target = (
                        (rect[0].as_u64().context("level x")?
                            + rect[2].as_u64().context("level width")? / 2)
                            as u16,
                        (rect[1].as_u64().context("level y")?
                            + rect[3].as_u64().context("level height")? / 2)
                            as u16,
                    );
                    if destination["rooms"].as_array().is_none_or(|r| r.is_empty()) {
                        destination["rooms"] = json!([{"rect": destination["rect"], "grid": null}]);
                    }
                    let waypoint = approach_waypoint(&map, from, target, 3)?;
                    let camera = &state["client"]["camera"];
                    let cam = crate::rules::camera::Camera::new(
                        crate::rules::camera::FrameSize::play(),
                        crate::rules::camera::OpenMode::NONE,
                        crate::rules::camera::moving_to_client(
                            camera["x16"].as_u64().context("camera x")? as u32,
                            camera["y16"].as_u64().context("camera y")? as u32,
                        ),
                        (0, 0),
                    );
                    let at = walk_point(&cam, waypoint);
                    self.pointer(
                        at,
                        Some(UiEvent::Press {
                            button: PointerButton::Left,
                            at,
                        }),
                    );
                    self.pointer(
                        at,
                        Some(UiEvent::Release {
                            button: PointerButton::Left,
                            at,
                        }),
                    );
                    self.step(24);
                }
                bail!("walklevel: did not enter level {level} after 64 clicks")
            }
            // `autoplay-host` only moves when stepped and each pass is a
            // tick: it is always paused between commands.
            Command::StepTicks(n) => {
                self.step(n);
                Ok(Some(
                    json!({"k": "ok", "cmd": "step-ticks", "passes": self.passes}),
                ))
            }
            Command::Pause => ok("pause"),
            Command::Resume => ok("resume"),
            Command::Shot(_) => anyhow::bail!(
                "shot: autoplay-host draws no picture; use play --offscreen --control"
            ),
            Command::Move(p) => {
                protocol::pointer(&mut self.ui(), p, None);
                ok("move")
            }
            Command::Press(button, at) => {
                protocol::pointer(&mut self.ui(), at, Some(UiEvent::Press { button, at }));
                ok("press")
            }
            Command::Release(button, at) => {
                protocol::pointer(&mut self.ui(), at, Some(UiEvent::Release { button, at }));
                ok("release")
            }
            Command::Key(vk) => {
                let codes = protocol::key(&mut self.ui(), vk);
                self.keys.extend(codes);
                ok("key")
            }
            Command::RawKey(vk) => {
                let codes = protocol::key_codes(vk);
                if !codes.is_empty() {
                    self.keys.extend(codes);
                    self.window = true;
                }
                ok("rawkey")
            }
            Command::Query(names) => {
                let state = self.state()?;
                Ok(Some(protocol::query_reply(&names, &state)))
            }
            Command::ClickUi(id) => {
                let els = self.ui_elements();
                let at = crate::ui::ui_ids::find(&els, &id)
                    .map_err(anyhow::Error::msg)?
                    .at;
                let button = PointerButton::Left;
                protocol::pointer(&mut self.ui(), at, Some(UiEvent::Press { button, at }));
                protocol::pointer(&mut self.ui(), at, Some(UiEvent::Release { button, at }));
                Ok(Some(
                    json!({"k": "ok", "cmd": "click", "id": id, "at": [at.x, at.y]}),
                ))
            }
            Command::InteractUnit(sel) | Command::UseUnit(sel) => {
                let immediate = matches!(c, Command::UseUnit(_));
                // NPC talk reach is wider than the client's pending melee
                // reach. Leave room for their footprint, then use the normal
                // interact sender; the server checks its own talk range.
                let reach = if sel.unit_type == 1 { 4 } else { 3 };
                // Space advances each greeting line and preserves the menu
                // after the last line. Mouse skips end the interaction.
                let mut skips = 0;
                while self
                    .ui()
                    .original
                    .as_ref()
                    .is_some_and(|o| o.npc_dialog_lines().is_some())
                {
                    if skips == 64 {
                        bail!("interactunit: NPC greeting did not finish after 64 skips");
                    }
                    self.key(0x20);
                    self.step(4);
                    skips += 1;
                }
                let mut approach = 0;
                let mut walking = false;
                let unit = loop {
                    // Clear panels before sampling the camera: movement can
                    // continue during these passes, so an earlier projection
                    // would send the waypoint click to a different cell.
                    if walking {
                        self.key(0x20);
                        self.step(4);
                    }
                    let world = self.app.world();
                    let model = world.resource::<BridgeResource>().0.world();
                    let cam = world
                        .get_resource::<crate::world_view::WorldViewState>()
                        .and_then(|s| s.anchor)
                        .map(|a| {
                            a.camera(
                                crate::rules::camera::FrameSize::play(),
                                crate::rules::camera::OpenMode::NONE,
                            )
                        })
                        .or_else(|| input_script::script_camera(model, None))
                        .context("interactunit: no camera")?;
                    let loaded = select_interact_unit(model, &cam, &sel).ok();
                    if immediate {
                        break loaded.context("useunit: no such unit in client")?;
                    }
                    // The bot reads the host's map and unit locations; all
                    // movement still goes through ordinary pointer input.
                    let state = self.state()?;
                    let units = &state["snap"]["units"];
                    let map = self.map()?;
                    let presets: Vec<Value> = map["levels"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .flat_map(|l| l["rooms"].as_array().into_iter().flatten())
                        .flat_map(|r| r["presets"].as_array().into_iter().flatten().cloned())
                        .collect();
                    let (target, cell) = snapshot_approach(units, &cam, &sel)
                        .or_else(|_| snapshot_approach(&json!(presets), &cam, &sel))?;
                    let player = units
                        .as_array()
                        .context("snapshot units")?
                        .iter()
                        .find(|u| u["ut"] == 0 && u["g"] == state["local"])
                        .context("interactunit: no local player")?;
                    let from = (
                        player["x"].as_u64().context("player x")? as u16,
                        player["y"].as_u64().context("player y")? as u16,
                    );
                    if from.0.abs_diff(cell.0).max(from.1.abs_diff(cell.1)) <= reach
                        && loaded == Some(target)
                    {
                        break target;
                    }
                    if approach == 64 {
                        bail!("interactunit: approach did not reach target after 64 clicks");
                    }
                    if !walking {
                        walking = true;
                        continue;
                    }
                    let waypoint = approach_waypoint(&map, from, cell, reach)?;
                    let at = walk_point(&cam, waypoint);
                    self.pointer(
                        at,
                        Some(UiEvent::Press {
                            button: PointerButton::Left,
                            at,
                        }),
                    );
                    self.pointer(
                        at,
                        Some(UiEvent::Release {
                            button: PointerButton::Left,
                            at,
                        }),
                    );
                    self.step(24);
                    approach += 1;
                };
                // A walk may already have opened this NPC's menu. Keep that
                // interaction, rather than closing it with another click.
                let menu = self.ui().original.as_ref().and_then(|o| o.npc_menu());
                if immediate || unit.unit_type == 1 {
                    let keep_menu = !immediate
                        && menu
                            .as_ref()
                            .is_some_and(|m| m.guid == unit.guid && !m.talking);
                    if !keep_menu {
                        if menu.is_some() {
                            self.key(0x1B);
                            self.step(1);
                        }
                        self.app
                            .world_mut()
                            .resource_mut::<crate::world_view::WorldViewState>()
                            .interact
                            .note(&[crate::controls::click::ClickOut::Pend(None)], true);
                        self.app
                            .world_mut()
                            .resource_mut::<BridgeResource>()
                            .0
                            .interact(unit)?;
                    }
                    return Ok(Some(
                        json!({"k":"ok", "cmd": if immediate {"useunit"} else {"interactunit"}, "unit":[unit.unit_type, unit.guid]}),
                    ));
                }
                if !menu
                    .as_ref()
                    .is_some_and(|m| m.guid == unit.guid && unit.unit_type == 1 && !m.talking)
                {
                    if menu.is_some() {
                        self.key(0x1B);
                        self.step(1);
                    }
                    let mut state = self
                        .app
                        .world_mut()
                        .resource_mut::<crate::world_view::WorldViewState>();
                    state.interact.note(
                        &[crate::controls::click::ClickOut::Pend(Some(
                            crate::controls::click::Pending {
                                code: 0x13,
                                target: unit,
                            },
                        ))],
                        true,
                    );
                }
                Ok(Some(
                    json!({"k": "ok", "cmd": "interactunit", "unit": [unit.unit_type, unit.guid]}),
                ))
            }
            Command::ClickUnit(sel) => {
                let (unit, at) = {
                    let world = self.app.world();
                    let model = world.resource::<BridgeResource>().0.world();
                    let cam = world
                        .get_resource::<crate::world_view::WorldViewState>()
                        .and_then(|s| s.anchor)
                        .map(|a| {
                            a.camera(
                                crate::rules::camera::FrameSize::play(),
                                crate::rules::camera::OpenMode::NONE,
                            )
                        })
                        .or_else(|| input_script::script_camera(model, None))
                        .context("clickunit: no camera")?;
                    input_script::unit_point(model, &cam, &sel).map_err(anyhow::Error::msg)?
                };
                // Hover must settle before the press, as in T2's input script.
                self.pointer(at, None);
                self.step(1);
                let button = sel.button;
                self.pointer(at, Some(UiEvent::Press { button, at }));
                self.pointer(at, Some(UiEvent::Release { button, at }));
                Ok(Some(
                    json!({"k": "ok", "cmd": "clickunit", "unit": [unit.unit_type, unit.guid], "at": [at.x, at.y]}),
                ))
            }
            Command::Ui => {
                let els = self.ui_elements();
                Ok(Some(super::autoplay_front::ui_reply("game", &els)))
            }
            Command::Text(_) => bail!("text: the front end's name box only (--front)"),
            Command::State => self.state().map(Some),
            Command::Map => protocol::map(&self.server).map(Some),
            Command::Quit => Ok(None),
        }
    }
}

/// `d2-client autoplay-host...`: starts the game, prints the hello line and
/// serves commands until `quit` or the end of stdin.
pub fn serve(args: &[String]) -> Result<()> {
    let a = parse_args(args)?;
    if let Some(res) = a.res {
        crate::rules::camera::FrameSize::set_play(res)?;
    }
    let mut session = if a.front {
        Session::Front(Box::new(FrontPhase::start(&a)?))
    } else {
        Session::Game(Box::new(Host::start(&a)?))
    };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let hello = match &session {
        Session::Front(f) => {
            json!({"k": "hello", "format": FORMAT, "tick_rate": d2_sim::tick::TickRate::GAME.hz(), "passes": 0, "front": f.screen()})
        }
        Session::Game(h) => {
            json!({"k": "hello", "format": FORMAT, "tick_rate": d2_sim::tick::TickRate::GAME.hz(), "passes": h.passes})
        }
    };
    writeln!(out, "{hello}")?;
    out.flush()?;
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let reply = match parse_command(t) {
            Ok(c) => match session.run(c) {
                Ok(Some(v)) => v,
                Ok(None) => break,
                // a read that cannot answer (no local player: the game
                // was left) is reported, the host keeps serving
                Err(e) => json!({"k": "error", "error": format!("{e:#}")}),
            },
            Err(e) => json!({"k": "error", "error": e}),
        };
        writeln!(out, "{reply}")?;
        out.flush()?;
    }
    Ok(())
}

/// A session: the front end until it starts the game (`--front`), then
/// the game.
pub(crate) enum Session {
    Front(Box<FrontPhase>),
    Game(Box<Host>),
}

impl Session {
    /// Runs one command; its reply, `None` for `quit`. A `step N` that
    /// starts the game on the front end runs the rest of its N in the
    /// game.
    pub(crate) fn run(&mut self, c: Command) -> Result<Option<Value>> {
        if c == Command::Quit {
            return Ok(None);
        }
        match self {
            Session::Game(h) => h.run(c),
            Session::Front(f) => match f.run(c)? {
                FrontStep::Reply(v) => Ok(Some(v)),
                FrontStep::Game(mut host, rest) => {
                    host.step(rest);
                    let v =
                        json!({"k": "ok", "cmd": "step", "passes": host.passes, "screen": "game"});
                    *self = Session::Game(host);
                    Ok(Some(v))
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Point, PointerButton};
    use crate::world_view::input_script::Step;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unloaded_target_selection_and_immediate_parser() {
        use crate::rules::camera::{moving_to_client, Camera, FrameSize, OpenMode};
        let camera = Camera::new(
            FrameSize::play(),
            OpenMode::NONE,
            moving_to_client(100 << 16, 100 << 16),
            (0, 0),
        );
        let Command::InteractUnit(sel) = parse_command("interactunit 1 198").unwrap() else {
            panic!("interaction")
        };
        let units = json!([
            {"ut":2,"cl":198,"g":1,"x":101,"y":100,"m":1},
            {"ut":1,"cl":199,"g":2,"x":101,"y":100,"m":1},
            {"ut":1,"cl":198,"g":3,"x":400,"y":400,"m":1}
        ]);
        assert_eq!(
            snapshot_approach(&units, &camera, &sel).unwrap(),
            (crate::bridge::world::UnitKey::new(1, 3), (400, 400))
        );
        assert!(snapshot_approach(&json!([]), &camera, &sel).is_err());
        let Command::UseUnit(immediate) = parse_command("useunit 1 198").unwrap() else {
            panic!("immediate interaction")
        };
        assert_eq!(sel, immediate);
        for bad in [
            "useunit",
            "useunit 9 198",
            "useunit 1 x",
            "useunit 1 198 0 -8",
        ] {
            assert!(parse_command(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn walking_projection_clicks_the_planned_cell_at_each_resolution() {
        use crate::rules::camera::{moving_to_client, Camera, FrameSize, OpenMode};
        for size in FrameSize::ALL {
            let camera = Camera::new(
                size,
                OpenMode::NONE,
                moving_to_client(5123 << 16, 5098 << 16),
                (0, 0),
            );
            for cell in [(5129, 5100), (5118, 5097), (5123, 5098)] {
                let at = walk_point(&camera, cell);
                assert_eq!(
                    crate::bridge::click::screen_to_world(&camera, at.x, at.y),
                    (i32::from(cell.0), i32::from(cell.1)),
                    "{size:?}: {cell:?}"
                );
            }
        }
    }

    #[test]
    fn approach_routes_around_walls_and_rejects_disconnected_targets() {
        let cells: String = (0..10)
            .flat_map(|y| {
                (0..10).map(move |x| {
                    if x == 4 && !(6..=8).contains(&y) {
                        '#'
                    } else {
                        '.'
                    }
                })
            })
            .collect();
        let map = json!({"levels":[{"rooms":[{"grid":{"rect":[0,0,10,10],"cells":cells}}]}]});
        for from in [(2, 2), (3, 2)] {
            let waypoint = approach_waypoint(&map, from, (8, 2), 3).unwrap();
            assert!(
                waypoint.1 >= 6,
                "must walk toward the gap even when beside the wall: {from:?} -> {waypoint:?}"
            );
        }
        // The local player's NO_PATH marker is rendered as a blocked cell.
        // Movement clears it before checking the destination footprint.
        let mut occupied = map.clone();
        let mut cells = occupied["levels"][0]["rooms"][0]["grid"]["cells"]
            .as_str()
            .unwrap()
            .as_bytes()
            .to_vec();
        cells[22] = b'#';
        occupied["levels"][0]["rooms"][0]["grid"]["cells"] =
            json!(String::from_utf8(cells).unwrap());
        assert!(approach_waypoint(&occupied, (2, 2), (8, 2), 3).unwrap().1 >= 6);
        let blocked =
            json!({"levels":[{"rooms":[{"grid":{"rect":[0,0,10,10],"cells":"#".repeat(100)}}]}]});
        assert!(approach_waypoint(&blocked, (2, 2), (8, 2), 3).is_err());
        let narrow: String = (0..10)
            .flat_map(|y| (0..10).map(move |x| if x == 4 && y != 7 { '#' } else { '.' }))
            .collect();
        let narrow = json!({"levels":[{"rooms":[{"grid":{"rect":[0,0,10,10],"cells":narrow}}]}]});
        assert!(approach_waypoint(&narrow, (2, 2), (8, 2), 3).is_err());
        assert_eq!(approach_waypoint(&map, (2, 2), (2, 6), 4).unwrap(), (2, 2));
    }

    #[test]
    fn interaction_can_select_offscreen_units_and_breaks_ties_by_key() {
        use crate::bridge::world::{ClientUnit, ClientWorld, UnitKey};
        use crate::rules::camera::{moving_to_client, Camera, FrameSize, OpenMode};
        let mut world = ClientWorld::default();
        let camera = Camera::new(
            FrameSize::play(),
            OpenMode::NONE,
            moving_to_client(100 << 16, 100 << 16),
            (0, 0),
        );
        let Command::InteractUnit(sel) = parse_command("interactunit 1 154").unwrap() else {
            panic!("interaction")
        };
        for guid in [8, 3] {
            let mut u = ClientUnit::new(UnitKey::new(1, guid));
            u.class = 154;
            u.position = Some((200, 200));
            u.mode = 1;
            world.units.insert(u.key, u);
        }
        assert!(
            input_script::unit_point(&world, &camera, &sel).is_err(),
            "outside frame"
        );
        assert_eq!(
            select_interact_unit(&world, &camera, &sel),
            Ok(UnitKey::new(1, 3))
        );
        world.units.get_mut(&UnitKey::new(1, 8)).unwrap().position = Some((101, 101));
        assert_eq!(
            select_interact_unit(&world, &camera, &sel),
            Ok(UnitKey::new(1, 8))
        );
        let Command::InteractUnit(missing) = parse_command("interactunit 1 148").unwrap() else {
            panic!("interaction")
        };
        assert!(select_interact_unit(&world, &camera, &missing).is_err());
    }

    #[test]
    fn options_and_commands_parse() {
        let a = parse_args(&args(&["--new", "amazon", "Bot", "--seed", "7"])).unwrap();
        assert_eq!(a.new, Some(("amazon".into(), "Bot".into())));
        assert_eq!(a.seed, Some(7));
        assert_eq!(a.res, None);
        let a = parse_args(&args(&["--new", "amazon", "Bot", "--res", "1068x600"])).unwrap();
        assert_eq!(a.res, Some(crate::rules::camera::FrameSize::WIDE));
        assert!(parse_args(&args(&["--new", "amazon", "Bot", "--res", "1024x768"])).is_err());
        assert!(parse_args(&args(&[])).is_err(), "a character is needed");
        assert!(parse_args(&args(&["--save", "a", "--new", "ama", "B"])).is_err());
        assert_eq!(parse_command("step 5"), Ok(Command::Step(5)));
        assert_eq!(parse_command("walklevel 2"), Ok(Command::WalkLevel(2)));
        for bad in [
            "walklevel",
            "walklevel 0",
            "walklevel 137",
            "walklevel x",
            "walklevel 2 3",
        ] {
            assert!(parse_command(bad).is_err(), "{bad}");
        }
        assert_eq!(
            parse_command("press L 400 300"),
            Ok(Command::Press(
                PointerButton::Left,
                Point { x: 400, y: 300 }
            ))
        );
        assert_eq!(
            parse_command("release r 1 2"),
            Ok(Command::Release(PointerButton::Right, Point { x: 1, y: 2 }))
        );
        assert_eq!(parse_command("key 1"), Ok(Command::Key(0x31)));
        assert_eq!(parse_command("key esc"), Ok(Command::Key(0x1B)));
        assert_eq!(parse_command("state"), Ok(Command::State));
        assert!(matches!(
            parse_command("query player.pos, unit(monster, *).mode"),
            Ok(Command::Query(n)) if n.len() == 2
        ));
        assert!(parse_command("query player.speed").is_err());
        assert_eq!(
            parse_command("click ui:menu.single_player"),
            Ok(Command::ClickUi("menu.single_player".into()))
        );
        assert_eq!(parse_command("ui"), Ok(Command::Ui));
        let Command::InteractUnit(sel) = parse_command("interactunit 1 154,148").unwrap() else {
            panic!("interaction")
        };
        assert_eq!(sel.unit_type, 1);
        assert_eq!(&sel.classes[..usize::from(sel.count)], &[154, 148]);
        for line in [
            "clickunit 1 154",
            "rclickunit 2 119,145 2 -12",
            "clickunit 1 *",
        ] {
            let Step::ClickUnit(sel) = input_script::parse_step(line, (800, 600)).unwrap() else {
                panic!("unit step")
            };
            assert_eq!(parse_command(line), Ok(Command::ClickUnit(sel)));
        }
        for line in [
            "interactunit 1",
            "interactunit 6 154",
            "interactunit 1 154 0 -8",
            "clickunit 1",
            "clickunit 6 154",
            "clickunit 1 bad",
            "clickunit 1 154 2",
            "rclickunit 1 1,2,3,4,5,6,7,8,9",
        ] {
            assert!(parse_command(line).is_err(), "{line}");
        }
        assert_eq!(
            parse_command("text Probe"),
            Ok(Command::Text("Probe".into()))
        );
        let a = parse_args(&args(&["--front", "--save-dir", "s", "--seed", "1"])).unwrap();
        assert!(a.front && a.save_dir == Some(PathBuf::from("s")));
        assert!(
            parse_args(&args(&["--front"])).is_err(),
            "--front needs --save-dir"
        );
        assert!(parse_args(&args(&["--front", "--save-dir", "s", "--new", "ama", "B"])).is_err());
        for bad in [
            "step",
            "step -1",
            "press X 1 2",
            "move 800 0",
            "key nope",
            "fly",
            "click 1 2",
            "click ui:",
            "click ui:a b",
            "text",
        ] {
            assert!(parse_command(bad).is_err(), "{bad}");
        }
    }
}
