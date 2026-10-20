//! `d2-client soak`: the play client, headless (the app `play` builds, on
//! `MinimalPlugins`, as the play smoke test runs it), driven faster than real
//! time by seeded random input or by a replayed input log, with the soak's
//! checks after every frame: panics, hangs (no server tick), the server state's
//! invariants and the client model against the server. D2RUST only; a finding
//! is a D2RUST bug or a D2RUST gap, never a fidelity claim.
//!
//! Every input goes through the client: UI events (the same the window
//! sends), the bridge's interaction, or the C→S intents the client's
//! panels send. The start pokes (`warp`, the item kit) are the poke
//! tool's, run before the first input. The game
//! clock steps at the game rate (60 Hz, with rational millisecond deadlines);
//! nothing reads the wall clock but the hang guard's report.

pub mod checks;
pub mod gen;
pub mod log;
pub mod roundtrip;

use std::io::Write as _;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use bevy::prelude::*;

use super::play::{add_live_client, LiveClient};
use super::play_start::{self, CliStart};
use super::single_player::{self, Character, GameData, Link};
use super::state_dump::StepClock;
use crate::bridge::world::{UnitKey, OBJECT};
use crate::bridge::BridgeResource;
use crate::ui::panel::ActionId;
use crate::ui::{Point, PointerButton, UiEvent};
use crate::world_view::WorldViewUi;

use checks::{Desync, GuidHistory, Violation};
use gen::Gen;
use log::{Act, SoakLog, Start};

/// Frames without a server tick after which the run is hung.
pub const HANG_FRAMES: u32 = 250;

/// Frames the join may take before the run gives up.
const JOIN_FRAMES: u32 = 2000;

/// The soak's item kit: put on the ground around the player
/// before the first input, so pick-up, belt, equip and portal inputs have
/// something to move. (code, dx, dy).
pub const KIT: &[(&str, i32, i32)] = &[
    ("tsc", 1, 0),
    ("tsc", 2, 0),
    ("tbk", 0, 1),
    ("isc", 1, 1),
    ("hp1", 2, 1),
    ("hp1", 3, 1),
    ("mp1", 0, 2),
    ("mp1", 1, 2),
    ("rvs", 2, 2),
    ("lbl", 3, 2),
    ("hax", 0, 3),
    ("buc", 1, 3),
    ("cap", 2, 3),
    ("rin", 3, 3),
    ("box", 3, 0),
];

type Server = super::save::SharedLink<StepClock>;

/// One finding of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// `panic`, `hang`, `server-stopped`, `model`, `guid`, `vitals`,
    /// `room`, `item`, `list`, `desync`, `setup`.
    pub kind: String,
    /// Stable signature (no step or frame): what a reduction keeps.
    pub sig: String,
    pub step: u32,
    pub frame: u32,
    pub detail: String,
}

impl Finding {
    fn from_violation(v: Violation, step: u32, frame: u32) -> Self {
        Self {
            kind: v.kind.to_owned(),
            sig: v.sig,
            step,
            frame,
            detail: v.detail,
        }
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"kind\":{},\"sig\":{},\"step\":{},\"frame\":{},\"detail\":{}}}",
            json_str(&self.kind),
            json_str(&self.sig),
            self.step,
            self.frame,
            json_str(&self.detail)
        )
    }
}

fn json_str(s: &str) -> String {
    serde_json::Value::String(s.to_owned()).to_string()
}

/// `soak`'s options.
#[derive(Clone, Debug, Default)]
pub struct SoakArgs {
    pub start: Start,
    /// Generator seed (`--seed`); ignored with `--replay`.
    pub seed: u64,
    /// Frames to run.
    pub steps: u32,
    /// `--replay LOG`: run a log's actions instead of the generator.
    pub replay: Option<PathBuf>,
    /// `--log-out FILE`: the input log, written as the run goes (flushed
    /// per line, so a killed run leaves its log).
    pub log_out: Option<PathBuf>,
    /// `--report FILE`: the findings, one JSON line each, and a summary.
    pub report: Option<PathBuf>,
    /// `--keep-going`: do not stop at the first finding.
    pub keep_going: bool,
    /// `--rate N`: percent of frames with an input (default 30).
    pub rate: Option<u32>,
    pub game_dir: Option<PathBuf>,
    /// `--roundtrip-at S,S,...`: a save/load round trip before those
    /// steps; the run goes on in the reloaded game.
    pub roundtrip_at: Vec<u32>,
    /// `--save-dir DIR`: where the run's saves go (default: a folder in
    /// the system temp dir). Never the game install.
    pub save_dir: Option<PathBuf>,
}

pub fn parse_args(args: &[String]) -> Result<SoakArgs> {
    let mut a = SoakArgs {
        start: Start {
            kit: true,
            ..Start::default()
        },
        seed: 1,
        steps: 3000,
        ..SoakArgs::default()
    };
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--save" => a.start.save = Some(value()?.clone()),
            "--new" => a.start.class = Some(value()?.clone()),
            "--game-seed" => a.start.game_seed = Some(value()?.parse().context("--game-seed")?),
            "--difficulty" => {
                let v = value()?;
                a.start.difficulty = single_player::parse_difficulty(v)
                    .with_context(|| format!("--difficulty {v}"))?;
            }
            "--warp" => a.start.warp = Some(value()?.parse().context("--warp")?),
            "--no-kit" => a.start.kit = false,
            "--seed" => a.seed = value()?.parse().context("--seed")?,
            "--steps" => a.steps = value()?.parse().context("--steps")?,
            "--replay" => a.replay = Some(PathBuf::from(value()?)),
            "--log-out" => a.log_out = Some(PathBuf::from(value()?)),
            "--report" => a.report = Some(PathBuf::from(value()?)),
            "--keep-going" => a.keep_going = true,
            "--rate" => a.rate = Some(value()?.parse().context("--rate")?),
            "--game-dir" => a.game_dir = Some(PathBuf::from(value()?)),
            "--roundtrip-at" => {
                for v in value()?.split(',') {
                    a.roundtrip_at.push(v.parse().context("--roundtrip-at")?);
                }
                a.roundtrip_at.sort_unstable();
            }
            "--save-dir" => a.save_dir = Some(PathBuf::from(value()?)),
            other => bail!("soak: unknown option {other}"),
        }
    }
    Ok(a)
}

/// The headless play app and its server.
struct Rig {
    app: App,
    server: Server,
    saver: super::save::SaveHandle,
    ms: Arc<AtomicU32>,
    frames: u32,
    difficulty: u8,
}

impl Rig {
    /// The character of a start: its save (as `play --save` resolves it)
    /// or a new one.
    fn character(data: &GameData, start: &Start, game_dir: Option<&Path>) -> Result<Character> {
        Ok(match (&start.save, &start.class) {
            (Some(s), _) => {
                play_start::resolve(
                    &CliStart {
                        save: Some(PathBuf::from(s)),
                        new: None,
                        save_dir: None,
                        difficulty: start.difficulty,
                        hardcore: false,
                    },
                    data,
                    game_dir,
                    None,
                    None,
                )?
                .character
            }
            (None, class) => {
                single_player::new_character(class.as_deref().unwrap_or("sorceress"), "Soak")?
                    .with_difficulty(start.difficulty)
            }
        })
    }

    /// The play app on `character`, its server writing saves to
    /// `save_to` (`play::run`'s wiring: `save::share`).
    fn start(
        data: &GameData,
        character: Character,
        game_seed: Option<u32>,
        save_to: PathBuf,
    ) -> Result<Self> {
        let GameData::Live(live) = data.clone();
        let seed = single_player::game_seed(&character, game_seed);
        let ms = Arc::new(AtomicU32::new(super::state_dump::START_MS));
        let speeds = single_player::walk_speeds(data, &character)?;
        let mut base = super::save::base_save(&character);
        let hardcore = base.header.status & d2_formats::d2s::status::HARDCORE != 0;
        if hardcore {
            base.header.status |= d2_formats::d2s::status::HARDCORE;
        }
        let (mut link, started) = single_player::start_with(
            data.clone(),
            seed,
            character.clone(),
            StepClock(ms.clone()),
        )?;
        if hardcore {
            link.with(|l| l.host_mut().game.events.action.hooks().x.hardcore = true)?;
        }
        let tables: Arc<dyn d2_formats::d2s::SaveTables + Send + Sync> =
            Arc::new(live.save.clone());
        let appearance = Some(Arc::new(
            super::save::appearance_tables(&live.tables.fixed).map_err(anyhow::Error::msg)?,
        ));
        let (server, saver) = super::save::share(link, base, tables, appearance, save_to)?;
        let mut app = App::new();
        app.insert_resource(crate::bridge::mirror::ScriptedClock(ms.clone()));
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .init_resource::<ButtonInput<MouseButton>>();
        app.insert_resource(saver.clone());
        add_live_client(
            &mut app,
            Box::new(server.clone()),
            LiveClient {
                data: &live,
                request: &character,
                start_flags: None,
                prices: started.prices,
                speeds,
                hardcore,
                automap_files: None,
                gpu: false,
            },
        )?;
        Ok(Self {
            app,
            server,
            saver,
            ms,
            frames: 0,
            difficulty: character.difficulty(),
        })
    }

    /// Frames until the server and the model both have the local player.
    fn join(&mut self) -> bool {
        for _ in 0..JOIN_FRAMES {
            let has = self
                .with(|l| single_player::local_player(&l.host().game).is_some())
                .unwrap_or(false);
            if has && self.world().local_player.is_some() {
                return true;
            }
            self.update();
        }
        false
    }

    /// Opens the Esc menu (the single-player pause: no server frame runs)
    /// and reads the live state the save will write.
    fn paused_live(&mut self) -> Result<super::save::Live, String> {
        if !self.paused() {
            self.queue(UiEvent::Action(ActionId(
                crate::controls::Action::GameMenu.index() as u16,
            )));
        }
        for _ in 0..2 {
            self.update();
        }
        self.with(|l| super::save::read_live(&mut l.host_mut().game).map_err(|e| e.to_string()))?
    }

    /// Save and Exit (C→S 0x69): frames until
    /// the app stops on the server's answer; the server's leave wrote the
    /// file.
    fn save_and_exit(&mut self) -> Result<PathBuf, String> {
        super::save::request_save_and_exit(self.bridge()).map_err(|e| e.to_string())?;
        for _ in 0..300 {
            if self.app.should_exit().is_some() {
                break;
            }
            self.update();
        }
        if self.app.should_exit().is_none() {
            return Err("Save and Exit: the app did not stop in 300 frames".into());
        }
        let p = self.saver.path().to_path_buf();
        if !p.exists() {
            return Err(format!("Save and Exit wrote no {}", p.display()));
        }
        Ok(p)
    }

    /// The load's own report: the lines of the action log that say a
    /// part of the save was not loaded.
    fn load_breaks(&self) -> Vec<String> {
        self.with(|l| {
            l.host_mut()
                .game
                .events
                .action
                .hooks()
                .x
                .log
                .iter()
                .filter(|l| {
                    l.contains("load failed") || l.contains("items:") || l.contains("quests ")
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
    }

    fn update(&mut self) {
        self.app.update();
        self.ms.store(
            super::state_dump::step_ms(d2_sim::tick::TickRate::GAME, self.frames + 1),
            Ordering::SeqCst,
        );
        self.frames += 1;
    }

    fn with<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Link<StepClock>) -> R + Send + 'static,
    ) -> Result<R, String> {
        self.server.with(f).map_err(|e| e.0)
    }

    fn bridge(&mut self) -> &mut crate::bridge::Bridge<crate::bridge::mirror::DynLink> {
        &mut self
            .app
            .world_mut()
            .resource_mut::<BridgeResource>()
            .into_inner()
            .0
    }

    fn world(&self) -> &crate::bridge::world::ClientWorld {
        self.app.world().resource::<BridgeResource>().0.world()
    }

    fn queue(&mut self, e: UiEvent) {
        self.app
            .world_mut()
            .non_send_mut::<WorldViewUi>()
            .queue
            .0
            .push(e);
    }

    /// Runs one action through the client. Returns a note when the client
    /// itself refused it (a bridge error); a refusal is not a finding.
    fn apply(&mut self, act: &Act) -> Option<String> {
        let r: Result<(), String> = match *act {
            Act::Click { right, x, y } => {
                let button = if right {
                    PointerButton::Right
                } else {
                    PointerButton::Left
                };
                let at = Point { x, y };
                self.queue(UiEvent::CursorMoved(at));
                self.queue(UiEvent::Press { button, at });
                self.queue(UiEvent::Release { button, at });
                Ok(())
            }
            Act::Move { x, y } => {
                self.queue(UiEvent::CursorMoved(Point { x, y }));
                Ok(())
            }
            Act::Key(k) => {
                self.queue(UiEvent::Action(ActionId(k.index() as u16)));
                Ok(())
            }
            Act::Interact { ut, guid } => {
                let key = UnitKey {
                    unit_type: ut,
                    guid,
                };
                match self.world().units.get(&key) {
                    // The click path never targets an object its model marks
                    // unselectable; an old log's interaction with one is not
                    // sent (it could move the object to a mode without
                    // client graphics, a fatal in the model).
                    Some(u) if ut == OBJECT && !u.flag_4 => Ok(()),
                    Some(_) => self
                        .bridge()
                        .interact(key)
                        .map(|_| ())
                        .map_err(|e| e.to_string()),
                    None => Err("no such unit in the model".into()),
                }
            }
            Act::Pick { guid, cursor } => self.send(&crate::bridge::items::pick(guid, cursor)),
            Act::Drop { guid } => self.send(&crate::bridge::items::drop(guid)),
            Act::Insert { guid, x, y, page } => {
                self.send(&crate::bridge::items::insert(guid, x, y, page))
            }
            Act::Remove { guid } => self.send(&crate::bridge::items::remove(guid)),
            Act::Equip { guid, body } => self.send(&crate::bridge::items::equip(guid, body)),
            Act::Belt { guid, slot } => self.send(&crate::bridge::items::to_belt(guid, slot)),
            Act::UseGrid { guid } => {
                let (x, y) = self
                    .world()
                    .local_player
                    .and_then(|k| self.world().units.get(&k))
                    .and_then(|u| u.position)
                    .unwrap_or((0, 0));
                self.send(&crate::bridge::items::use_grid(
                    guid,
                    u32::from(x),
                    u32::from(y),
                ))
            }
            Act::UseBelt { guid } => self.send(&crate::bridge::items::use_belt(guid)),
            Act::Waypoint { guid, level } => {
                let mut m = vec![0x49];
                m.extend_from_slice(&guid.to_le_bytes());
                m.extend_from_slice(&level.to_le_bytes());
                self.bridge()
                    .send_bytes(&m)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            Act::Wait(_) | Act::RoundTrip => Ok(()),
        };
        r.err()
    }

    /// The single-player pause (UI state 9; the Esc menu; or 11): no server
    /// frame runs, so no tick is not a hang.
    fn paused(&self) -> bool {
        self.app
            .world()
            .non_send::<WorldViewUi>()
            .original
            .as_ref()
            .is_some_and(|u| u.is_open(9) || u.is_open(11))
    }

    fn send<M: d2_proto::FixedMessage>(&mut self, m: &M) -> Result<(), String> {
        self.bridge().send(m).map(|_| ()).map_err(|e| e.to_string())
    }

    /// The model's receive log: every unhandled,
    /// rejected and discarded S→C message so far, as (signature,
    /// detail) with a running count per signature.
    fn model_log(&self) -> std::collections::BTreeMap<String, (u64, String)> {
        let l = self.app.world().resource::<BridgeResource>().0.log();
        let mut m = std::collections::BTreeMap::new();
        for (id, n) in &l.unowned {
            m.insert(
                format!("model:unhandled:{id:02X}"),
                (*n, format!("{n} unhandled S→C 0x{id:02X}")),
            );
        }
        // `dropped` (a unit message for a unit the model lacks at
        // receive) is the client's own rule,
        // not a finding.
        for r in &l.rejected {
            // The error's variant, not its numbers.
            let what: String = format!("{:?}", r.error)
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let e = m
                .entry(format!("model:rejected:{:02X}:{what}", r.id))
                .or_insert((0, String::new()));
            e.0 += 1;
            e.1 = format!("rejected S→C 0x{:02X}: {:?}", r.id, r.error);
        }
        for d in &l.discarded {
            let e = m
                .entry(format!("model:discarded:{:02X}", d.first))
                .or_insert((0, String::new()));
            e.0 += 1;
            e.1 = format!("discarded {} bytes from S→C 0x{:02X}", d.bytes, d.first);
        }
        m
    }
}

/// What a run did.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub steps: u32,
    pub server_frame: u32,
    pub actions: u32,
    pub refused: u32,
    /// Round trips run.
    pub roundtrips: u32,
    /// Save and Exits the input clicked (each reloaded).
    pub exits: u32,
    pub findings: Vec<Finding>,
}

fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| "non-string panic".into())
}

/// The first line of a panic message without its numbers (addresses,
/// GUIDs, coordinates), so one bug has one signature.
fn panic_sig(msg: &str) -> String {
    let first = msg.lines().next().unwrap_or("");
    let mut s: String = first
        .chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect();
    s.truncate(160);
    format!("panic:{s}")
}

/// Runs a soak: `actions` (a replay) or the generator, for `steps`
/// frames after the join and the start pokes. `log_line` receives every
/// action line as it runs (the input log).
pub fn soak(
    data: &GameData,
    args: &SoakArgs,
    replay: Option<&SoakLog>,
    mut log_line: impl FnMut(&str),
) -> Outcome {
    let mut out = Outcome::default();
    let start = replay.map_or(&args.start, |l| &l.start);
    let header = SoakLog {
        start: start.clone(),
        seed: if replay.is_some() { 0 } else { args.seed },
        acts: Vec::new(),
    }
    .header();
    log_line(&header);
    let finding = |kind: &str, sig: String, step, frame, detail: String| Finding {
        kind: kind.to_owned(),
        sig,
        step,
        frame,
        detail,
    };
    let save_dir = args.save_dir.clone().unwrap_or_else(|| {
        std::env::temp_dir().join(format!("d2rust-soak-{}", std::process::id()))
    });
    if let Err(e) = std::fs::create_dir_all(&save_dir) {
        out.findings.push(finding(
            "setup",
            "setup:save-dir".into(),
            0,
            0,
            e.to_string(),
        ));
        return out;
    }
    let mut generation = 0u32;
    let rig = catch_unwind(AssertUnwindSafe(|| -> Result<Rig> {
        let c = Rig::character(data, start, args.game_dir.as_deref())?;
        Rig::start(data, c, start.game_seed, save_dir.join("Soak0.d2s"))
    }));
    let mut rig = match rig {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            out.findings.push(finding(
                "setup",
                "setup:start".into(),
                0,
                0,
                format!("{e:#}"),
            ));
            return out;
        }
        Err(p) => {
            let m = panic_text(&p);
            out.findings.push(finding("panic", panic_sig(&m), 0, 0, m));
            return out;
        }
    };
    // The join.
    let joined = catch_unwind(AssertUnwindSafe(|| rig.join()));
    match joined {
        Ok(true) => {}
        Ok(false) => {
            out.findings.push(finding(
                "setup",
                "setup:join".into(),
                0,
                rig.frames,
                format!("no local player after {JOIN_FRAMES} frames"),
            ));
            return out;
        }
        Err(p) => {
            let m = panic_text(&p);
            out.findings
                .push(finding("panic", panic_sig(&m), 0, rig.frames, m));
            return out;
        }
    }
    // The start pokes (poke directives, `app::poke::apply_now`).
    let mut pokes: Vec<String> = Vec::new();
    if let Some(level) = start.warp {
        pokes.push(format!("warp {level}"));
    }
    for _ in 0..10 {
        rig.update();
    }
    for p in pokes {
        if let Err(e) = run_poke(&rig, &p) {
            out.findings
                .push(finding("setup", "setup:poke".into(), 0, rig.frames, e));
            return out;
        }
        for _ in 0..60 {
            rig.update();
        }
    }
    if start.kit {
        for (code, dx, dy) in KIT {
            // A kit item the ground refuses (a blocked cell) is left out.
            let off = |c: &str, d: i32| {
                if d == 0 {
                    format!("@{c}")
                } else {
                    format!("@{c}+{d}")
                }
            };
            let p = format!("item {code} {} {}", off("x", *dx), off("y", *dy));
            if let Err(e) = run_poke(&rig, &p) {
                eprintln!("soak: kit: {e}");
            }
        }
        for _ in 0..10 {
            rig.update();
        }
    }

    let waypoint_classes: Vec<u32> = {
        let GameData::Live(live) = data;
        live.waypoints
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.operatefn == 23)
            .map(|(i, _)| i as u32)
            .collect()
    };
    let mut gen = Gen::new(args.seed, waypoint_classes);
    if let Some(r) = args.rate {
        gen.rate = r;
    }
    let mut pending: std::collections::VecDeque<(u32, Act)> = replay
        .map(|l| l.acts.iter().cloned().collect())
        .unwrap_or_default();
    let mut desync = Desync::default();
    let mut guids = GuidHistory::default();
    let mut seen_sigs = std::collections::BTreeSet::new();
    let mut model_seen = rig.model_log();
    let (mut last_frame, mut idle) = (0u32, 0u32);
    let mut wait = 0u32;

    let mut roundtrips: std::collections::VecDeque<u32> =
        args.roundtrip_at.iter().copied().collect();
    'run: for step in 0..args.steps {
        out.steps = step + 1;
        // This step's inputs.
        let mut acts = Vec::new();
        if replay.is_some() {
            while pending.front().is_some_and(|(s, _)| *s <= step) {
                acts.push(pending.pop_front().unwrap().1);
            }
        } else if roundtrips.front().is_some_and(|&s| s <= step) {
            roundtrips.pop_front();
            acts.push(Act::RoundTrip);
        } else if wait > 0 {
            wait -= 1;
        } else if let Some(a) = gen.next(rig.world()) {
            if let Act::Wait(n) = a {
                wait = n;
            }
            acts.push(a);
        }
        if let Some(i) = acts.iter().position(|a| *a == Act::RoundTrip) {
            log_line(&SoakLog::line(step, &Act::RoundTrip));
            acts.remove(i);
            generation += 1;
            let r = catch_unwind(AssertUnwindSafe(|| {
                round_trip(rig, data, &save_dir, generation)
            }));
            let (next, violations) = match r {
                Ok(x) => x,
                Err(p) => {
                    let m = panic_text(&p);
                    out.findings
                        .push(finding("panic", panic_sig(&m), step, last_frame, m));
                    out.server_frame = last_frame;
                    return out;
                }
            };
            for v in violations {
                let f = Finding::from_violation(v, step, last_frame);
                if seen_sigs.insert(f.sig.clone()) {
                    out.findings.push(f);
                }
            }
            match next {
                Some(r) => rig = r,
                None => {
                    out.server_frame = last_frame;
                    return out;
                }
            }
            // The new game's frames and model start over.
            last_frame = 0;
            idle = 0;
            desync = Desync::default();
            guids = GuidHistory::default();
            model_seen = rig.model_log();
            out.roundtrips += 1;
        }
        for a in &acts {
            log_line(&SoakLog::line(step, a));
            out.actions += 1;
            let r = catch_unwind(AssertUnwindSafe(|| rig.apply(a)));
            match r {
                Ok(Some(_refused)) => out.refused += 1,
                Ok(None) => {}
                Err(p) => {
                    let m = panic_text(&p);
                    out.findings
                        .push(finding("panic", panic_sig(&m), step, last_frame, m));
                    break 'run;
                }
            }
        }
        if let Err(p) = catch_unwind(AssertUnwindSafe(|| rig.update())) {
            let m = panic_text(&p);
            out.findings
                .push(finding("panic", panic_sig(&m), step, last_frame, m));
            break;
        }
        // A Save and Exit the input clicked (the Esc menu): the game ends
        // as `play` ends it; the run goes on in the character reloaded
        // from that save.
        if rig.app.should_exit().is_some() {
            generation += 1;
            let GameData::Live(live) = data;
            let path = rig.saver.path().to_path_buf();
            let difficulty = rig.difficulty;
            let next = roundtrip::read_file(live, &path, difficulty).and_then(|s| {
                let c = Character::Save(
                    Box::new(s),
                    d2_server::adapters::character::LoadContext {
                        difficulty,
                        map_seed_applies: false,
                    },
                );
                let mut r = Rig::start(
                    data,
                    c,
                    None,
                    save_dir.join(format!("Soak{generation}x.d2s")),
                )
                .map_err(|e| format!("{e:#}"))?;
                if r.join() {
                    Ok(r)
                } else {
                    Err("no local player".into())
                }
            });
            match next {
                Ok(r) => rig = r,
                Err(e) => {
                    out.findings.push(finding(
                        "roundtrip",
                        "roundtrip:exit-reload".into(),
                        step,
                        last_frame,
                        e,
                    ));
                    break;
                }
            }
            last_frame = 0;
            idle = 0;
            desync = Desync::default();
            guids = GuidHistory::default();
            model_seen = rig.model_log();
            out.exits += 1;
            continue;
        }
        // The checks.
        let checked = rig.with(|l| checks::server_check(&l.host().game));
        let (violations, view) = match checked {
            Ok(x) => x,
            Err(e) => {
                out.findings.push(finding(
                    "server-stopped",
                    "server-stopped".into(),
                    step,
                    last_frame,
                    e,
                ));
                break;
            }
        };
        if view.frame > last_frame || rig.paused() {
            last_frame = last_frame.max(view.frame);
            idle = 0;
        } else {
            idle += 1;
            if idle >= HANG_FRAMES {
                out.findings.push(finding(
                    "hang",
                    "hang:no-tick".into(),
                    step,
                    last_frame,
                    format!("no server tick in {HANG_FRAMES} frames"),
                ));
                break;
            }
        }
        let mut found: Vec<Finding> = violations
            .into_iter()
            .chain(guids.observe(&view.units))
            .chain(desync.observe(rig.world(), &view))
            .map(|v| Finding::from_violation(v, step, last_frame))
            .collect();
        let model_now = rig.model_log();
        for (sig, (n, detail)) in &model_now {
            if model_seen.get(sig).is_none_or(|(m, _)| m < n) {
                found.push(finding(
                    "model",
                    sig.clone(),
                    step,
                    last_frame,
                    detail.clone(),
                ));
            }
        }
        model_seen = model_now;
        for f in found {
            // One report per signature per run.
            if seen_sigs.insert(f.sig.clone()) {
                out.findings.push(f);
            }
        }
        if !out.findings.is_empty() && !args.keep_going {
            break;
        }
    }
    out.server_frame = last_frame;
    if std::env::var_os("SOAK_DUMP_ITEMS").is_some() {
        let w = rig.world();
        for i in crate::bridge::items::items(w) {
            eprintln!("model item: {i:?}");
        }
        if let Ok((_, v)) = rig.with(|l| checks::server_check(&l.host().game)) {
            for i in v.items.values() {
                eprintln!("server item: {i:?}");
            }
        }
    }
    out
}

/// Save and Exit `rig`, reload the file, compare the live state and
/// a second save with the first; the run goes on in a third game loaded
/// from the second save. `n`: the round trip's number (file names).
fn round_trip(mut rig: Rig, data: &GameData, dir: &Path, n: u32) -> (Option<Rig>, Vec<Violation>) {
    let GameData::Live(live) = data;
    let mut out = Vec::new();
    let fail = |what: &str, e: String| Violation {
        units: Vec::new(),
        kind: "roundtrip",
        sig: format!("roundtrip:{what}"),
        detail: e,
    };
    let difficulty = rig.difficulty;
    let loaded = |s: d2_formats::d2s::D2s| {
        Character::Save(
            Box::new(s),
            d2_server::adapters::character::LoadContext {
                difficulty,
                map_seed_applies: false,
            },
        )
    };
    let before = match rig.paused_live() {
        Ok(l) => l,
        Err(e) => return (None, vec![fail("read-live", e)]),
    };
    let first = match rig
        .save_and_exit()
        .and_then(|p| roundtrip::read_file(live, &p, difficulty))
    {
        Ok(s) => s,
        Err(e) => return (None, vec![fail("first-save", e)]),
    };
    drop(rig);
    let mut again = match Rig::start(
        data,
        loaded(first.clone()),
        None,
        dir.join(format!("Soak{n}a.d2s")),
    ) {
        Ok(r) => r,
        Err(e) => return (None, vec![fail("reload", format!("{e:#}"))]),
    };
    if !again.join() {
        return (None, vec![fail("reload-join", "no local player".into())]);
    }
    for b in again.load_breaks() {
        out.push(fail("load-log", b));
    }
    match again.paused_live() {
        Ok(after) => out.extend(roundtrip::diff_live(&before, &after)),
        Err(e) => out.push(fail("read-live-after", e)),
    }
    let second = match again
        .save_and_exit()
        .and_then(|p| roundtrip::read_file(live, &p, difficulty))
    {
        Ok(s) => s,
        Err(e) => {
            out.push(fail("second-save", e));
            return (None, out);
        }
    };
    out.extend(roundtrip::diff_saves(&first, &second));
    drop(again);
    let next = Rig::start(
        data,
        loaded(second),
        None,
        dir.join(format!("Soak{n}b.d2s")),
    )
    .ok()
    .and_then(|mut r| r.join().then_some(r));
    if next.is_none() {
        out.push(fail("continue", "the third game did not start".into()));
    }
    (next, out)
}

fn run_poke(rig: &Rig, text: &str) -> Result<(), String> {
    let toks: Vec<&str> = text.split_ascii_whitespace().collect();
    let op = d2_sim::poke::parse_op(&toks)?;
    let r = rig.with(move |l| super::poke::apply_now(&mut l.host_mut().game, &op))?;
    match r {
        d2_sim::poke::PokeResult::Ok(_) => Ok(()),
        r => Err(format!("poke `{text}`: {r:?}")),
    }
}

/// `d2-client soak...`: exit 0 no finding, 1 findings, 3 error.
pub fn run(args: &SoakArgs) -> Result<i32> {
    let dir = args
        .game_dir
        .clone()
        .or_else(|| std::env::var_os("D2_GAME_DIR").map(PathBuf::from));
    let data = GameData::select(dir.as_deref())?;
    let replay = match &args.replay {
        Some(p) => Some(
            SoakLog::parse(
                &std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
            )
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("parsing {}", p.display()))?,
        ),
        None => None,
    };
    let mut log = match &args.log_out {
        Some(p) => Some(std::io::BufWriter::new(
            std::fs::File::create(p).with_context(|| format!("creating {}", p.display()))?,
        )),
        None => None,
    };
    // A soak run only reports panics; the default hook would print each
    // one as well (kept: its location is the most useful line).
    let t0 = std::time::Instant::now();
    let out = soak(&data, args, replay.as_ref(), |line| {
        if let Some(w) = log.as_mut() {
            let _ = writeln!(w, "{line}");
            let _ = w.flush();
        }
    });
    let secs = t0.elapsed().as_secs_f64();
    let summary = format!(
        "{{\"summary\":true,\"steps\":{},\"server_frame\":{},\"actions\":{},\"refused\":{},\"roundtrips\":{},\"exits\":{},\"findings\":{},\"wall_s\":{:.1}}}",
        out.steps,
        out.server_frame,
        out.actions,
        out.refused,
        out.roundtrips,
        out.exits,
        out.findings.len(),
        secs
    );
    let mut text = String::new();
    for f in &out.findings {
        text.push_str(&f.to_json());
        text.push('\n');
    }
    text.push_str(&summary);
    text.push('\n');
    match &args.report {
        Some(p) => std::fs::write(p, &text).with_context(|| format!("writing {}", p.display()))?,
        None => print!("{text}"),
    }
    for f in &out.findings {
        eprintln!(
            "soak: step {} frame {}: {} {}: {}",
            f.step, f.frame, f.kind, f.sig, f.detail
        );
    }
    eprintln!("soak: {summary}");
    Ok(i32::from(!out.findings.is_empty()))
}
