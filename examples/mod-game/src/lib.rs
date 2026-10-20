//! An example mod game: the shape every mod game built on D2RUST has
//! (`docs/mods/mod-game.md`, `docs/plans/mods/seams.md` MS16).
//!
//! A mod game is its own binary. It depends on the D2RUST crates, installs
//! its overrides as a `ModRules` in the `Ruleset` of every game it builds,
//! chooses the layers its data is opened with and keeps its characters in
//! its own save folder. The D2RUST exe never loads it: players start the
//! mod game directly.
//!
//! This example is synthetic: it carries no mod data and no real mod's
//! names. Its one rule is a skill body in a "do" slot past D2RUST's own
//! table, so D2RUST's own skills are unchanged. Its layers are the folders
//! under `mods/` of the native folder it is pointed at.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use d2_client::app::server_thread::ThreadLink;
use d2_client::app::single_player::{
    self, D2sCodec, GameBuilder, GameData, GameSource, Layers, Link, Saves, Started,
};
use d2_client::bridge::link::{SendQueue, ServerLink};
use d2_server::seams::Clock;
use d2_sim::rules::{ModRules, Ruleset, SkillFn, SkillHost, SkillProvider};
use d2_sim::units::UnitId;

/// The example's name, in logs and the dev console.
pub const NAME: &str = "example-mod-game";

/// The "do" slot of the example's skill body: past D2RUST's own table, so
/// it adds a body instead of replacing one.
pub const EXAMPLE_DO_SLOT: u16 = 240;

/// The example's skill bodies: one added "do" body that reports that it
/// ran (returns 1) and changes nothing else.
#[derive(Debug, Default)]
pub struct ExampleSkills;

impl SkillProvider for ExampleSkills {
    fn has(&self, kind: SkillFn, index: u16) -> bool {
        kind == SkillFn::Do && index == EXAMPLE_DO_SLOT
    }

    fn srvdo(
        &self,
        _w: &mut dyn SkillHost,
        index: u16,
        _u: UnitId,
        _skill: i32,
        _lvl: i32,
        _charge: bool,
        _item: bool,
        _aim: bool,
    ) -> Option<i32> {
        (index == EXAMPLE_DO_SLOT).then_some(1)
    }
}

/// The example's overrides: its name and its skill bodies.
#[derive(Debug, Default)]
pub struct ExampleRules {
    skills: ExampleSkills,
}

impl ModRules for ExampleRules {
    fn name(&self) -> &str {
        NAME
    }

    fn skills(&self) -> Option<&dyn SkillProvider> {
        Some(&self.skills)
    }
}

/// Where the example reads the player's files and keeps its saves.
#[derive(Debug, Clone)]
pub struct Options {
    /// The player's files: an install folder or a converted native folder
    /// with the example's layers.
    pub source: GameSource,
    /// The example's own save folder (never D2RUST's).
    pub saves: PathBuf,
}

impl Options {
    /// Reads `--game-dir DIR`, or `--native DIR [--layer NAME]...`, and
    /// `--saves DIR` (default `mod-game-saves` in the working folder).
    pub fn parse(args: impl IntoIterator<Item = String>) -> anyhow::Result<Options> {
        let mut game_dir = None;
        let mut native = None;
        let mut layers = Vec::new();
        let mut saves = PathBuf::from("mod-game-saves");
        let mut args = args.into_iter();
        while let Some(a) = args.next() {
            let mut value = || {
                args.next()
                    .ok_or_else(|| anyhow::anyhow!("{a} needs a value"))
            };
            match a.as_str() {
                "--game-dir" => game_dir = Some(PathBuf::from(value()?)),
                "--native" => native = Some(PathBuf::from(value()?)),
                "--layer" => layers.push(value()?),
                "--saves" => saves = PathBuf::from(value()?),
                other => anyhow::bail!("unknown argument {other}"),
            }
        }
        let source = match (native, game_dir) {
            (Some(dir), _) => GameSource::Native(dir, Layers::List(layers)),
            (None, Some(dir)) if layers.is_empty() => GameSource::Archives(dir),
            (None, Some(_)) => anyhow::bail!("--layer needs --native: an install has no layers"),
            (None, None) => anyhow::bail!("pass --game-dir DIR or --native DIR"),
        };
        Ok(Options { source, saves })
    }
}

/// The example's game builder on `data`: its rules and its saves.
pub fn builder(data: GameData, saves: PathBuf) -> GameBuilder {
    GameBuilder::new(data)
        .rules(Ruleset::Mod(Arc::new(ExampleRules::default())))
        .saves(Saves::new(saves, Arc::new(D2sCodec)))
}

/// The server's milliseconds per frame step of [`StepClock`].
pub const STEP_MS: u32 = 40;

/// A host clock that moves only when stepped, so a headless run is the
/// same on every machine.
#[derive(Debug, Clone)]
pub struct StepClock(Arc<AtomicU32>);

impl Default for StepClock {
    fn default() -> StepClock {
        StepClock(Arc::new(AtomicU32::new(1000)))
    }
}

impl StepClock {
    /// Moves the clock on by `ms`.
    pub fn step(&self, ms: u32) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Starts the example's game on its own server thread behind `clock` and
/// joins its character: the client's create request (C→S 0x67), then its
/// join (C→S 0x6B), one stepped server frame each.
pub fn start_and_join(
    builder: GameBuilder,
    clock: &StepClock,
) -> anyhow::Result<(ThreadLink<Link<StepClock>>, Started)> {
    let request = single_player::create_request_for(&builder.character);
    let (mut link, started) = builder.start(clock.clone())?;
    link.send(SendQueue::System, &request.encode())?;
    link.pump()?;
    clock.step(STEP_MS);
    link.pump()?;
    link.receive();
    link.send(SendQueue::System, &[0x6B])?;
    clock.step(STEP_MS);
    link.pump()?;
    link.receive();
    Ok((link, started))
}

/// Runs `frames` stepped server frames; the number of game ticks run.
pub fn run_frames(
    link: &mut ThreadLink<Link<StepClock>>,
    clock: &StepClock,
    frames: u32,
) -> anyhow::Result<u32> {
    let mut ticks = 0;
    for _ in 0..frames {
        clock.step(STEP_MS);
        if link.pump()?.ticked {
            ticks += 1;
        }
        link.receive();
    }
    Ok(ticks)
}

/// The name of the rules the game behind `link` runs under (`None` for
/// D2RUST's own).
pub fn rules_name(link: &mut ThreadLink<Link<StepClock>>) -> anyhow::Result<Option<String>> {
    Ok(link.with(|l| {
        let hooks = l.host_mut().game.events.action.hooks();
        hooks.rules.mod_rules().map(|r| r.name().to_owned())
    })?)
}
