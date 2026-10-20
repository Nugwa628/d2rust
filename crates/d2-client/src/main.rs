//! d2-client entry point.
//!
//! Usage:
//!   d2-client [play]     [--game-dir DIR] [--res 800x600|640x480|1068x600] [--anim-dir DIR] [--seed N] [--frames N] [--difficulty normal|nightmare|hell] [--save FILE.d2s | --new CLASS NAME [--save-dir DIR]] [--native DIR] [--source native|mpq]
//!   d2-client view       [--ds1 PATH] [--wall-base N] [--frames N]
//!   d2-client verify     [--case NAME]... [--cases DIR] [--perturb N]
//!   d2-client verify     [--ds1 PATH] [--wall-base N] [--view L,T,W,H] [--out DIR] [--perturb N]
//!   d2-client cpu-render [--ds1 PATH] [--wall-base N] [--view L,T,W,H] [--out FILE]
//!   d2-client play       ... --dump-draws DIR [--at-tick N[,M...]] [--dump-image] [--dump-native WxH[,WxH...]] [--input SCRIPT] [--frame-schedule FILE] [--no-sound] [--registry FILE]
//!   d2-client play       ... [--poke "F DIRECTIVE ARGS"]... [--poke-file FILE]
//!                        (pokes: F is the absolute server frame; file ticks
//!                        are relative to the join)
//!   d2-client play       ... --offscreen [--size WxH] [--shot-at T[,T..]] [--shot-every N] [--shot-dir DIR] [--display-clock fixed:FPS|phases:P[,P..]] [--shot-phase P]
//!   d2-client play       ... --control stdio|pipe
//!   d2-client play       --online [--server ID] [--game-dir DIR]
//!                        (online play, builds with the `online` feature:
//!                        the servers come from `servers.toml` next to the exe)
//!   d2-client animation-proof --out LOCAL_DIR [--game-dir DIR] [--class 0..6]
//!   d2-client bake-anims --game-dir DIR --out DIR [--jobs N] [--plan] [--limit N]
//!   d2-client animation-build --source-pack FILE --palette FILE --ramps FILE --out DIR [--loop]
//!   d2-client facts-compare BASE_DIR D2RUST_DIR [--ignore COL,...]
//!   d2-client sweep FAMILY [--filter TEXT] [--jobs N] [--out DIR] [--game-dir DIR | --native DIR [--mod NAME]] [--save-failures DIR]
//!   d2-client soak [--save FILE.d2s | --new CLASS] [--warp LEVEL] [--seed N] [--steps N] [--replay LOG] [--log-out LOG] [--report FILE] [--keep-going]
//!   d2-client autoplay-host (--save FILE.d2s | --new CLASS NAME) [--seed N] [--difficulty D] [--game-dir DIR] [--res WxH]
//!                        (the headless play client on a stdin/stdout line
//!                        protocol; tools/autoplay/ drives it)
//!   d2-client state-dump --save FILE.d2s [--seed N] [--difficulty D] --ticks T [--tick-rate HZ] [--every n] --out FILE [--game-dir DIR] [--date YYYY-MM-DD] [--poke "F DIRECTIVE ARGS"]... [--send "F NAME FIELD=VALUE..." | --send "F hex BYTES..."]... [--input SCRIPT] [--no-own-c2s ID[,ID]] [--packets FILE]
//!
//! `play` (the default) opens a window running the local single-player game: the
//! in-process server (`d2-server` host over the wired `d2-sim`) pumped
//! once per frame through the bridge, the world view composed by the GPU
//! compositor's render-graph node. With $D2_GAME_DIR set it reads the
//! game's `levels` and `objects` tables and generates its levels from the
//! player's DS1 / DT1 files; without $D2_GAME_DIR (or `--native DIR`) it
//! stops with an error naming D2_GAME_DIR (there is no invented game).
//! `--save` joins with a character save (read with the player's tables)
//! instead of a new sorceress. `--new CLASS NAME` joins with a new character of that class
//! (a name or 0–6: amazon, sorceress, necromancer, paladin, barbarian,
//! druid, assassin) and name (1–15 of A–Z, a–z, 0–9, `-`, `_`), held in
//! memory only: nothing is written to disk (a quick start that skips the
//! select and create screens). `--res 640x480` runs the game
//! in the 640 × 480 frame (resolution mode 0: camera, panels, hit tests,
//! window); `--res 1068x600` runs the 16:9 frame (the world fills the
//! width, the HUD and panels stay centred); without `--res` the frame is the
//! settings' Resolution row (16:9 by default).
//! `view` opens a window (pan: arrows/WASD, zoom: mouse wheel). `verify`
//! runs the render cases (`crates/d2-client/render-cases/*.toml`): per
//! case, CPU reference vs GPU, byte for byte; exit code 0 = all pass, 1 = a
//! failure or error, 2 = none failed but a GPU half is incomplete (no
//! adapter) or a scene case stops at a seam (the case lacks what the world
//! view needs; its camera is still checked). `verify --cases
//! crates/d2-client/capture-cases` runs the capture cases. With a map flag
//! (`--ds1`, `--wall-base`, `--view`, `--out`) it runs today's single-map
//! verify instead (default view: the whole map; exit 0 means identical).
//! `--perturb N` corrupts N reference pixels per case: each must fail with
//! exactly N. Every case (map and synthetic) runs on one headless compute
//! compositor. `cpu-render` writes the CPU reference image only.
//! `play --dump-draws DIR [--at-tick N[,M...]] [--dump-image]` writes the
//! rendering facts of the first drawn frame at server tick N or later
//! (default 1) to DIR and exits (skips the front end); several ticks write
//! DIR/tick-N each, `--dump-image` adds the composed frame as `frame.png`,
//! `--dump-native WxH[,WxH...]` the frame as a window of each size shows
//! it, without and with the native UI pass (`world_view::native_ui_shots`).
//! `play --sound-log FILE` writes every sound request call.
//! `play --input "move X Y; wait N; click X Y; rclick X Y"` plays the
//! steps as pointer input, timed in server ticks, in place of the window's
//! pointer.
//! `play --offscreen` (with `--save` or `--new`) runs the game in a hidden
//! window that never takes focus, at the player's resolution and video
//! settings with dev tools on, in a `--size` window (1920x1080 by default);
//! `--shot-at` / `--shot-every` write the presented picture at those server
//! ticks as PNGs in `--shot-dir` (`world_view::shots`). It ends after
//! `--frames N`, or after the last `--shot-at` shot. `--display-clock
//! fixed:FPS` or `phases:0,0.25,0.5,0.75` drives its clock from the frame
//! count, so two runs draw the same frames (`world_view::display_clock`);
//! with phases, `--shot-phase P` shoots the frame at phase P of each tick.
//! `--mute` (any subcommand) or `D2RUST_MUTE=1` opens no audio output
//! device: the game's sound logic runs, nothing is heard (`mute`). The
//! variable wins over the flags; `--offscreen` mutes unless `--sound` is
//! given. The player's options and saves are not changed. `--no-sound`
//! mutes the same way and also takes the sound device away from the game
//! itself (its sound init never runs).
//! `play --control stdio|pipe` (with `--save` or `--new`) drives the game
//! over the `autoplay-1` line protocol on stdin / stdout or the named pipe
//! `\\.\pipe\d2rust-<pid>`: pause, step ticks, input, reads and shots
//! (`app::control`). Off unless given; never a network port. With
//! `--offscreen` the channel's `quit` (or stdin closing) ends the run.
//! `facts-compare` reports the first difference between two facts dumps:
//! exit 0 match, 1 diverged, 2 partial, 3 error.
//! `state-dump` runs the game `play --save X --seed N` builds, headless, for
//! T server ticks and writes one `state-1` game-state snapshot per tick
//! (`--game-dir` or $D2_GAME_DIR).
//!
//! Game files are read from `--game-dir DIR`, else $D2_GAME_DIR, else the
//! exe's folder or the current folder when it holds `d2data.mpq`
//! (`launch::game_dir`; `docs/PLAYTEST.md`). A panic or a fatal error
//! writes `D2RUST-crash.log` next to the exe (`launch`). Output images go under the
//! gitignored `game/` folder by default; they contain game graphics and must
//! never be committed.
// The shipped (release) exe opens no console window: its output goes to
// `Documents\D2RUST\logs\game.log`, and a fatal error shows a message box
// (`launch`). Debug builds keep the console.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use d2_client::map::{self, cpu};
use d2_client::verify::{
    self,
    map::{archives, file_stem},
};

const DEFAULT_DS1: &str = r"data\global\tiles\ACT1\TOWN\townN1.ds1";
const DEFAULT_WALL_BASE: i32 = 80;

struct Options {
    ds1: String,
    wall_base: i32,
    view: Option<cpu::View>,
    out: Option<PathBuf>,
    probe: Option<(i32, i32)>,
    /// Debug: corrupt this many reference pixels per case, to prove
    /// `verify` fails with exactly that count.
    perturb: usize,
    /// `verify`: a map flag (`--ds1`, `--wall-base`, `--view`, `--out`) was
    /// given, so run today's single-map verify instead of the case files.
    map_flags: bool,
    /// `verify`: case directory (default `crates/d2-client/render-cases`).
    case_dir: Option<PathBuf>,
    /// `verify`: run only these cases (file stems); default all.
    cases: Vec<String>,
    /// `view`, `play`: close after this many frames (smoke test).
    frames: Option<u32>,
    /// `play --no-dev`: no dev tools.
    no_dev: bool,
    /// `play --online`: start at the online login screen (the launcher's
    /// "Play online"); needs a build with the `online` feature.
    online: bool,
    /// `play --online --server ID`: the server of `servers.toml` to log in
    /// to (the first one without it).
    server: Option<String>,
    /// `play --dev-start SLOT`: a saved dev slot to start from.
    dev_start: Option<PathBuf>,
    /// `play`: the fixed game seed (`--seed N`); `None`: a save's map
    /// seed, else a drawn seed for a front-end game or the default
    /// (`single_player::game_seed_or`).
    seed: Option<u32>,
    /// `play --save`: the character save the join loads.
    save: Option<PathBuf>,
    /// `play --difficulty normal|nightmare|hell|0-2`.
    difficulty: u8,
    /// `play --new CLASS NAME`: a new character, held in memory.
    new: Option<(String, String)>,
    /// `play --native DIR`: a converted native-assets folder.
    native: Option<PathBuf>,
    /// `play --source native|mpq` (`mpq`: debug builds only).
    source: Option<String>,
    /// `play --new`: the folder the new character's `<name>.d2s` goes in.
    save_dir: Option<PathBuf>,
    hardcore: bool,
    /// `play --dump-draws DIR`: the rendering facts export.
    dump_draws: Option<PathBuf>,
    /// `play --at-tick N[,M...]`: each dump's first server tick, strictly
    /// increasing.
    at_tick: Option<Vec<u64>>,
    /// `play --dump-image`: each dump also writes `frame.png`.
    dump_image: bool,
    /// `play --dump-native WxH[,WxH...]`: each dump also writes the frame
    /// as a window of each size shows it, without and with the native UI
    /// pass (`world_view::native_ui_shots`).
    dump_native: Vec<(u32, u32)>,
    /// `play --sound-log FILE`: every sound request call.
    sound_log: Option<PathBuf>,
    /// `play --frame-schedule FILE`: the frame schedule and host clock a
    /// check run follows.
    frame_schedule: Option<PathBuf>,
    /// `play --no-sound`: no sound device.
    no_sound: bool,
    /// `play --registry FILE`: the Options values a check run starts from.
    registry: Option<PathBuf>,
    /// `play --audio-dump FILE [--audio-ticks N]`: audio mixed without a device.
    audio_dump: Option<PathBuf>,
    device_ends: Option<PathBuf>,
    async_collects: Option<PathBuf>,
    /// `play --dump-loading FILE`: every loading-screen present.
    dump_loading: Option<PathBuf>,
    audio_ticks: Option<u64>,
    /// `play --input SCRIPT`: scripted pointer input.
    input: Option<Vec<d2_client::world_view::input_script::Step>>,
    /// `play --res 800x600|640x480|1068x600`: the play frame (default 800 × 600).
    res: Option<d2_client::rules::camera::FrameSize>,
    /// `play --poke "<f> <directive> ..."` (repeatable) and `--poke-file
    /// FILE`, in the order given.
    pokes: Vec<d2_client::app::poke::Entry>,
    /// `play --send "<f> <Name> <field>=<value>..." | "<f> hex <bytes>"`
    /// (repeatable).
    sends: Vec<d2_client::app::send::SendEntry>,
    /// `play --offscreen`: the game in a hidden window (`world_view::shots`).
    offscreen: bool,
    /// `play --offscreen --size WxH`: the hidden window's size.
    size: Option<(u32, u32)>,
    /// `play --offscreen --shot-at T[,T..]`: shots at these server ticks.
    shot_at: Vec<u64>,
    /// `play --offscreen --shot-every N`: a shot every N server ticks.
    shot_every: Option<u64>,
    /// `play --offscreen --shot-dir DIR`: where the shots go.
    shot_dir: Option<PathBuf>,
    /// `play --offscreen --display-clock CLOCK`: the frame count drives
    /// the clock.
    display_clock: Option<d2_client::world_view::display_clock::DisplayClock>,
    /// `play --offscreen --shot-phase P`: shots on the frame at phase P of
    /// their tick (a `--display-clock phases:` phase).
    shot_phase: Option<String>,
    /// `play --control stdio|pipe`: the line channel that drives the game.
    control: Option<d2_client::app::control::Transport>,
}

/// `800x600`, `640x480` or `1068x600` (the 16:9 frame): the play frames.
fn parse_res(s: &str) -> Result<d2_client::rules::camera::FrameSize> {
    use d2_client::rules::camera::FrameSize;
    FrameSize::parse(s).with_context(|| format!("--res {s}: use 800x600, 640x480 or 1068x600"))
}

fn parse_view(s: &str) -> Result<cpu::View> {
    let parts: Vec<i64> = s
        .split(',')
        .map(|p| p.trim().parse::<i64>())
        .collect::<Result<_, _>>()
        .context("--view expects L,T,W,H")?;
    let [left, top, width, height] = parts[..] else {
        bail!("--view expects 4 numbers: L,T,W,H");
    };
    Ok(cpu::View {
        left: left as i32,
        top: top as i32,
        width: u32::try_from(width).context("width")?,
        height: u32::try_from(height).context("height")?,
    })
}

fn parse_options(args: &[String]) -> Result<Options> {
    let mut o = Options {
        ds1: DEFAULT_DS1.to_owned(),
        wall_base: DEFAULT_WALL_BASE,
        view: None,
        out: None,
        probe: None,
        perturb: 0,
        map_flags: false,
        case_dir: None,
        cases: Vec::new(),
        frames: None,
        no_dev: false,
        online: false,
        server: None,
        dev_start: None,
        seed: None,
        difficulty: 0,
        save: None,
        new: None,
        native: None,
        source: None,
        save_dir: None,
        hardcore: false,
        res: None,
        dump_draws: None,
        at_tick: None,
        dump_image: false,
        dump_native: Vec::new(),
        sound_log: None,
        frame_schedule: None,
        no_sound: false,
        registry: None,
        audio_dump: None,
        device_ends: None,
        async_collects: None,
        dump_loading: None,
        audio_ticks: None,
        input: None,
        pokes: Vec::new(),
        sends: Vec::new(),
        offscreen: false,
        control: None,
        size: None,
        shot_at: Vec::new(),
        shot_every: None,
        shot_dir: None,
        display_clock: None,
        shot_phase: None,
    };
    let mut input: Option<String> = None;
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().with_context(|| format!("{flag} needs a value"));
        if matches!(flag.as_str(), "--ds1" | "--wall-base" | "--view" | "--out") {
            o.map_flags = true;
        }
        match flag.as_str() {
            "--case" => o.cases.push(value()?.clone()),
            "--cases" => o.case_dir = Some(PathBuf::from(value()?)),
            "--ds1" => o.ds1 = value()?.clone(),
            "--wall-base" => o.wall_base = value()?.parse().context("--wall-base")?,
            "--view" => o.view = Some(parse_view(value()?)?),
            "--out" => o.out = Some(PathBuf::from(value()?)),
            "--perturb" => o.perturb = value()?.parse().context("--perturb")?,
            "--frames" => o.frames = Some(value()?.parse().context("--frames")?),
            "--seed" => o.seed = Some(value()?.parse().context("--seed")?),
            "--hardcore" => o.hardcore = true,
            "--no-dev" => o.no_dev = true,
            "--online" => o.online = true,
            "--server" => o.server = Some(value()?.to_owned()),
            "--dev-start" => o.dev_start = Some(PathBuf::from(value()?)),
            "--dump-draws" => o.dump_draws = Some(PathBuf::from(value()?)),
            "--at-tick" => o.at_tick = Some(parse_ticks(value()?)?),
            "--dump-image" => o.dump_image = true,
            "--dump-native" => {
                o.dump_native = d2_client::world_view::native_ui_shots::parse_sizes(value()?)
                    .map_err(anyhow::Error::msg)?
            }
            "--sound-log" => o.sound_log = Some(PathBuf::from(value()?)),
            "--frame-schedule" => o.frame_schedule = Some(PathBuf::from(value()?)),
            "--no-sound" => o.no_sound = true,
            "--registry" => o.registry = Some(PathBuf::from(value()?)),
            "--dump-loading" => o.dump_loading = Some(PathBuf::from(value()?)),
            "--audio-dump" => o.audio_dump = Some(PathBuf::from(value()?)),
            "--device-ends" => o.device_ends = Some(PathBuf::from(value()?)),
            "--async-collects" => o.async_collects = Some(PathBuf::from(value()?)),
            "--audio-ticks" => o.audio_ticks = Some(value()?.parse().context("--audio-ticks")?),
            "--input" => input = Some(value()?.clone()),
            "--poke" => o
                .pokes
                .push(d2_client::app::poke::parse_poke_arg(value()?).map_err(anyhow::Error::msg)?),
            "--send" => o
                .sends
                .push(d2_client::app::send::parse_send_arg(value()?).map_err(anyhow::Error::msg)?),
            "--poke-file" => {
                let path = value()?;
                let text =
                    std::fs::read_to_string(path).with_context(|| format!("--poke-file {path}"))?;
                o.pokes.extend(
                    d2_client::app::poke::parse_poke_file(&text)
                        .map_err(|e| anyhow::anyhow!("--poke-file {path}: {e}"))?,
                );
            }
            "--save" => o.save = Some(PathBuf::from(value()?)),
            // Read before the options (`main`, `launch::game_dir`).
            // Both are read in `run`, before the options.
            "--game-dir" | "--anim-dir" => {
                value()?;
            }
            "--native" => o.native = Some(PathBuf::from(value()?)),
            "--source" => o.source = Some(value()?.clone()),
            "--difficulty" => {
                let v = value()?;
                o.difficulty =
                    d2_client::app::single_player::parse_difficulty(v).with_context(|| {
                        format!("--difficulty {v}: use normal, nightmare, hell or 0-2")
                    })?;
            }
            "--save-dir" => o.save_dir = Some(PathBuf::from(value()?)),
            "--res" => o.res = Some(parse_res(value()?)?),
            "--offscreen" => o.offscreen = true,
            "--size" => {
                o.size = Some(
                    d2_client::world_view::shots::parse_size(value()?)
                        .map_err(anyhow::Error::msg)?,
                )
            }
            "--shot-at" => {
                o.shot_at = d2_client::world_view::shots::parse_ticks(value()?)
                    .map_err(anyhow::Error::msg)?
            }
            "--shot-every" => {
                let v = value()?;
                o.shot_every = match v.parse::<u64>() {
                    Ok(n) if n > 0 => Some(n),
                    _ => bail!("--shot-every {v}: a whole number of ticks from 1"),
                }
            }
            "--shot-dir" => o.shot_dir = Some(PathBuf::from(value()?)),
            "--shot-phase" => o.shot_phase = Some(value()?.clone()),
            "--display-clock" => {
                o.display_clock = Some(
                    d2_client::world_view::display_clock::DisplayClock::parse(value()?)
                        .map_err(anyhow::Error::msg)?,
                )
            }
            "--control" => {
                o.control = Some(
                    d2_client::app::control::Transport::parse(value()?)
                        .map_err(anyhow::Error::msg)?,
                )
            }
            "--new" => {
                let class = value()?.clone();
                let name = it
                    .next()
                    .context("--new needs a class and a name: --new CLASS NAME")?
                    .clone();
                o.new = Some((class, name));
            }
            "--probe" => {
                let v = value()?;
                let (x, y) = v.split_once(',').context("--probe expects X,Y")?;
                o.probe = Some((x.trim().parse()?, y.trim().parse()?));
            }
            other => bail!("unknown option {other}"),
        }
    }
    // The script's points are frame pixels: checked against the frame
    // `--res` names, else the widest frame the settings can choose.
    if let Some(text) = input {
        use d2_client::rules::camera::FrameSize;
        let widest = FrameSize::ALL
            .into_iter()
            .max_by_key(|f| f.width * f.height);
        let f = o.res.or(widest).unwrap_or(FrameSize::D2RUST);
        o.input = Some(
            d2_client::world_view::input_script::parse_in(&text, (f.width, f.height))
                .map_err(|e| anyhow::anyhow!("--input: {e}"))?,
        );
    }
    if o.save.is_some() && o.new.is_some() {
        bail!("--save and --new cannot be combined");
    }
    if o.at_tick.is_some() && o.dump_draws.is_none() {
        bail!("--at-tick needs --dump-draws DIR");
    }
    if o.frame_schedule.is_some() && o.dump_draws.is_none() && o.audio_dump.is_none() {
        bail!("--frame-schedule needs --dump-draws DIR or --audio-dump FILE");
    }
    if o.dump_image && o.dump_draws.is_none() {
        bail!("--dump-image needs --dump-draws DIR");
    }
    if !o.dump_native.is_empty() && o.dump_draws.is_none() {
        bail!("--dump-native needs --dump-draws DIR");
    }
    let shots = !o.shot_at.is_empty() || o.shot_every.is_some();
    let offscreen_flags =
        shots || o.shot_dir.is_some() || o.size.is_some() || o.display_clock.is_some();
    if offscreen_flags && !o.offscreen {
        bail!("--size, --shot-at, --shot-every, --shot-dir and --display-clock need --offscreen");
    }
    if let Some(p) = &o.shot_phase {
        if !shots {
            bail!("--shot-phase P goes with --shot-at T[,T..] or --shot-every N");
        }
        let clock = o
            .display_clock
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("--shot-phase needs --display-clock phases:0,.."))?;
        clock.phase_frame(p).map_err(anyhow::Error::msg)?;
    }
    if shots != o.shot_dir.is_some() {
        bail!("--shot-dir DIR goes with --shot-at T[,T..] or --shot-every N");
    }
    if o.offscreen && o.save.is_none() && o.new.is_none() {
        bail!("--offscreen needs --save FILE.d2s or --new CLASS NAME");
    }
    if o.control.is_some() && o.save.is_none() && o.new.is_none() {
        bail!("--control needs --save FILE.d2s or --new CLASS NAME");
    }
    // A controlled run ends with the channel (`quit`, or stdin closing).
    if o.offscreen
        && o.control.is_none()
        && o.frames.is_none()
        && (o.shot_every.is_some() || o.shot_at.is_empty())
    {
        bail!("--offscreen needs an end: --frames N, --shot-at without --shot-every, or --control");
    }
    Ok(o)
}

/// `--at-tick N[,M...]`: one or more strictly increasing server ticks.
fn parse_ticks(s: &str) -> Result<Vec<u64>> {
    let ticks = s
        .split(',')
        .map(|t| t.trim().parse::<u64>().context("--at-tick"))
        .collect::<Result<Vec<_>>>()?;
    if !ticks.windows(2).all(|w| w[0] < w[1]) {
        bail!("--at-tick {s}: the ticks must be strictly increasing");
    }
    Ok(ticks)
}

fn cpu_render(o: Options) -> Result<()> {
    let archives = archives()?;
    let mut loaded = map::load(&archives, &o.ds1)?;
    for f in &loaded.missing_files {
        eprintln!("warning: tile file not found: {f}");
    }
    let mut layout = map::build(&loaded.ds1, &loaded.library, o.wall_base);
    let (dcc, dc6) = map::load_sprites(&archives)?;
    map::add_sprites(&mut layout, &mut loaded.library, &loaded.ds1, &dcc, &dc6);
    let missing: usize = layout.missing.values().sum();
    println!(
        "{}: {}x{} cells, {} tile keys, {} draw items, {} cells with missing tiles ({} keys)",
        o.ds1,
        loaded.ds1.width,
        loaded.ds1.height,
        loaded.library.len(),
        layout.items.len(),
        missing,
        layout.missing.len()
    );
    for it in layout
        .items
        .iter()
        .filter(|it| it.source == map::layout::Source::Sprite)
    {
        let img = &loaded.library.images[it.image];
        let opaque = img.pixels.iter().filter(|&&p| p != 0).count();
        println!(
            "  sprite at ({}, {}) size {}x{}, {opaque} opaque pixels",
            it.x, it.y, img.width, img.height
        );
    }
    if let Some((x, y)) = o.probe {
        println!("probe at screen ({x}, {y}), topmost first:");
        for (i, it, p) in layout.probe(&loaded.library, x, y) {
            let c = loaded.palette.colors[usize::from(p)];
            let at = (it.cell.1 * loaded.ds1.width as i32 + it.cell.0) as usize;
            let raw = match it.source {
                map::layout::Source::Floor(l) => {
                    format!("floor cell {:#010x}", loaded.ds1.floors[l][at])
                }
                map::layout::Source::Wall(l) => format!(
                    "wall cell {:#010x}, orientation cell {:#x}",
                    loaded.ds1.walls[l][at], loaded.ds1.orientations[l][at]
                ),
                map::layout::Source::Sprite => String::new(),
            };
            println!("    raw {raw}");
            println!(
                "  item {i}: {:?} cell {:?} key o{} m{} s{} at ({}, {}); index {p} = rgb({}, {}, {})",
                it.source,
                it.cell,
                it.key.orientation,
                it.key.main,
                it.key.sub,
                it.x,
                it.y,
                c.r,
                c.g,
                c.b
            );
        }
        return Ok(());
    }
    let bounds = layout.bounds.context("map has nothing to draw")?;
    let view = o.view.unwrap_or(cpu::View {
        left: bounds.x0,
        top: bounds.y0,
        width: (bounds.x1 - bounds.x0) as u32,
        height: (bounds.y1 - bounds.y0) as u32,
    });
    let indexed = cpu::render_indexed(&layout, &loaded.library, view);
    let rgba = cpu::to_rgba(&indexed, &loaded.palette);
    let out = o.out.unwrap_or_else(|| {
        PathBuf::from(format!(
            "game/renders/{}-cpu-wb{}.png",
            file_stem(&o.ds1),
            o.wall_base
        ))
    });
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image::save_buffer(
        &out,
        &rgba,
        view.width,
        view.height,
        image::ExtendedColorType::Rgba8,
    )
    .with_context(|| format!("writing {}", out.display()))?;
    println!(
        "wrote {} ({}x{}, view at {},{})",
        out.display(),
        view.width,
        view.height,
        view.left,
        view.top
    );
    Ok(())
}

fn verify(o: Options) -> Result<()> {
    if o.map_flags {
        // Today's verify, unchanged: one map from the command line.
        anyhow::ensure!(
            o.cases.is_empty() && o.case_dir.is_none(),
            "--case/--cases cannot be combined with --ds1/--wall-base/--view/--out"
        );
        let case = verify::case::MapCase {
            ds1: o.ds1,
            wall_base: o.wall_base,
            view: o.view.map(|v| (v.left, v.top, v.width, v.height)),
        };
        let mut gpu = verify::gpu::Wgpu::new();
        let report = verify::map::run_with("map", &case, o.out, o.perturb, &mut gpu);
        verify::print_report(&report);
        return match report.status {
            verify::Status::Pass => Ok(()),
            verify::Status::Fail(why) => bail!(why),
            verify::Status::Error(e) => bail!(e),
            verify::Status::GpuNotWired => bail!("map case: GPU half not wired"),
            verify::Status::SceneNotWired => bail!("map case: scene source not wired"),
            verify::Status::NoAdapter(why) => {
                eprintln!("map case: no GPU adapter: {why}");
                std::process::exit(2)
            }
        };
    }
    let dir = o.case_dir.unwrap_or_else(verify::default_case_dir);
    let mut cases = verify::load_dir(&dir).map_err(anyhow::Error::msg)?;
    if !o.cases.is_empty() {
        for name in &o.cases {
            anyhow::ensure!(
                cases.iter().any(|c| &c.name == name),
                "no case {name} in {}",
                dir.display()
            );
        }
        cases.retain(|c| o.cases.contains(&c.name));
    }
    println!("verify: {} cases from {}", cases.len(), dir.display());
    // One compute compositor on a headless adapter for every case (map and
    // synthetic alike), so the process opens one device; opened at the
    // first case.
    let mut gpu = verify::gpu::Wgpu::new();
    let mut announced = false;
    let mut summary = verify::Summary::default();
    for case in &cases {
        println!("case {} ({})", case.name, case.kind.name());
        if !std::mem::replace(&mut announced, true) {
            println!("GPU compositor: {}", gpu.open());
        }
        let report = match &case.kind {
            verify::CaseKind::Synthetic(s) => {
                verify::run_synthetic(&case.name, s, o.perturb, &mut gpu)
            }
            verify::CaseKind::Map(m) => {
                verify::map::run_with(&case.name, m, None, o.perturb, &mut gpu)
            }
            // A capture case: its camera is checked against the game
            // camera, then the frame goes through the world view
            // and `rules::OriginalView`; D2RUST does not yet read the draw
            // log into units, map and UI, so compare cases stop at that
            // seam (`scene_source::NotRecorded`).
            verify::CaseKind::Scene(sc) => verify::capture_case::run_capture(
                &case.name,
                sc,
                &verify::capture_case::repo_root(),
                o.perturb,
                &mut verify::capture_case::scene_source::WorldScene::new(
                    verify::capture_case::scene_source::NotRecorded,
                ),
                &mut gpu,
            ),
        };
        verify::print_report(&report);
        summary.add(&report.status);
    }
    println!("summary: {summary}");
    match summary.exit_code() {
        0 => Ok(()),
        code => std::process::exit(code),
    }
}

fn view(o: Options) -> Result<()> {
    let archives = Arc::new(archives()?);
    let result = d2_client::app::run(
        archives,
        d2_client::app::MapConfig {
            ds1_path: o.ds1,
            wall_base: o.wall_base,
        },
        d2_client::app::Mode::View {
            exit_after: o.frames,
        },
    );
    match result {
        bevy::app::AppExit::Success => Ok(()),
        bevy::app::AppExit::Error(code) => bail!("view exited with code {code}"),
    }
}

/// The game data of the options (`play`'s source choice).
fn select_data(
    o: &Options,
) -> Result<(
    d2_client::app::single_player::GameData,
    Option<PathBuf>,
    String,
)> {
    use d2_client::app::single_player;
    let choice = d2_client::assets::choose_source(
        o.native.as_deref(),
        o.source.as_deref(),
        |k| std::env::var(k).ok(),
        d2_client::assets::default_native_dir(),
    )
    .map_err(anyhow::Error::msg)?;
    let native = match &choice {
        d2_client::assets::SourceChoice::Native(dir) => Some(dir.clone()),
        _ => None,
    };
    let dir = std::env::var_os("D2_GAME_DIR").map(PathBuf::from);
    // D2RUST plays its own rules on the `base/` layer alone: it never reads
    // `mods/order.toml`. A mod game is its own binary and opens its own
    // layers (`docs/mods/mod-game.md`).
    let data = match &native {
        Some(dir) => single_player::GameData::open(&single_player::GameSource::Native(
            dir.clone(),
            single_player::Layers::base(),
        ))?,
        None => single_player::GameData::select(dir.as_deref())?,
    };
    let origin = match &native {
        Some(dir) => format!("native folder {}", dir.display()),
        None => "D2_GAME_DIR".to_owned(),
    };
    Ok((data, dir, origin))
}

/// `play`: the front end (main menu) first, then the game; Save and Exit
/// returns to the main menu, the game's window closing ends the
/// program. `--new`, `--save`, `--frames`, `--dump-draws` and `--offscreen`
/// skip the front end (a shortcut straight into the game).
///
/// One process, one window and one event loop for the whole session; the front end and each game are
/// states of one app (`app::session`), the game data is read once.
fn play(o: Options) -> Result<()> {
    if o.online {
        check_online(&o)?;
    }
    if let Some(res) = o.res {
        d2_client::rules::camera::FrameSize::set_play(res)?;
        println!("play: frame {} x {}", res.width, res.height);
    }
    let (data, dir, origin) = select_data(&o)?;
    if o.save.is_some()
        || o.new.is_some()
        || o.frames.is_some()
        || o.dump_draws.is_some()
        || o.dump_loading.is_some()
        || o.offscreen
    {
        let config = game_config(&o, data, dir, &origin, None, None, None)?;
        let save = config.save_path.clone();
        let end = d2_client::app::play::run(config)?;
        if let bevy::app::AppExit::Error(code) = end.exit {
            bail!("play exited with code {code}");
        }
        // Dev tools: a `load` starts again from the slot, in a
        // new process (a window's event loop runs once per process).
        return match (&end.relaunch, save) {
            (Some(slot), Some(save)) => relaunch_slot(&o, slot, &save),
            (Some(_), None) => {
                eprintln!("play: dev load: this game has no save file");
                Ok(())
            }
            _ => Ok(()),
        };
    }
    play_session(o, data, dir, origin)
}

/// `play --online`: the build has online play. The game opens at the
/// online login screen, which says what is wrong with the server list
/// (missing, damaged, no such server: the first one is used).
#[cfg(feature = "online")]
fn check_online(o: &Options) -> Result<()> {
    match d2_client::app::online::ServerList::load() {
        Ok(list) => match (list.pick(o.server.as_deref()), &o.server) {
            (Some(s), _) => println!("play: online, server {} ({})", s.name, s.address),
            (None, Some(id)) => eprintln!("play: the server list has no server {id:?}"),
            (None, None) => eprintln!("play: the server list is empty"),
        },
        Err(e) => eprintln!("play: {e}"),
    }
    Ok(())
}

#[cfg(not(feature = "online"))]
fn check_online(_: &Options) -> Result<()> {
    bail!("this build has no online play")
}

/// Dev tools: a `load` of a `play --save` shortcut: a new game process on the
/// slot's character (already the save file), seed and difficulty.
fn relaunch_slot(o: &Options, slot: &std::path::Path, save: &std::path::Path) -> Result<()> {
    let s = d2_client::app::dev::slots::read_slot(slot).map_err(anyhow::Error::msg)?;
    let exe = std::env::current_exe().context("the program's own path")?;
    let mut c = std::process::Command::new(exe);
    c.arg("play")
        .arg("--save")
        .arg(save)
        .arg("--seed")
        .arg(s.seed.to_string())
        .arg("--difficulty")
        .arg(s.difficulty.to_string())
        .arg("--dev-start")
        .arg(slot);
    if let Some(res) = o.res {
        c.arg("--res").arg(format!("{}x{}", res.width, res.height));
    }
    if let Some(d) = &o.save_dir {
        c.arg("--save-dir").arg(d);
    }
    if let Some(d) = &o.native {
        c.arg("--native").arg(d);
    }
    if let Some(src) = &o.source {
        c.arg("--source").arg(src);
    }
    println!("play: dev load: starting again from {}", slot.display());
    d2_client::launch::note(format!("dev load: {}", slot.display()));
    let status = c.status().context("starting the loaded game")?;
    if !status.success() {
        bail!("the loaded game exited with {status}");
    }
    Ok(())
}

/// Dev tools: a `load` in the session: the next game is the slot's
/// character (already the save file), seed and difficulty.
struct DevLoad {
    slot: PathBuf,
    save: PathBuf,
    seed: u32,
    difficulty: u8,
}

impl DevLoad {
    fn read(slot: &std::path::Path, save: PathBuf) -> Result<Self> {
        let s = d2_client::app::dev::slots::read_slot(slot).map_err(anyhow::Error::msg)?;
        println!("play: dev load: starting again from {}", slot.display());
        d2_client::launch::note(format!("dev load: {}", slot.display()));
        Ok(Self {
            slot: slot.to_path_buf(),
            save,
            seed: s.seed,
            difficulty: s.difficulty,
        })
    }
}

/// The session (`app::session`): one app whose front end builds the start
/// choice and whose game state runs it; back to the main menu after each
/// game.
fn play_session(
    o: Options,
    data: d2_client::app::single_player::GameData,
    dir: Option<PathBuf>,
    origin: String,
) -> Result<()> {
    use d2_client::app::front_host::{add_front_session, FrontArt, FrontStarter};
    use d2_client::app::front_start::{front_host_with, Entry, StartChoice, StartHandles};
    use d2_client::app::session::{self, GameEnder, GameStarter};
    use d2_client::ui::front_end::Outcome;
    use std::cell::RefCell;
    use std::rc::Rc;
    type Shared = Rc<
        RefCell<
            Option<(
                StartHandles,
                std::sync::Arc<std::sync::Mutex<Option<Outcome>>>,
            )>,
        >,
    >;
    let d2_client::app::single_player::GameData::Live(live) = &data;
    let live = live.clone();
    let saves = o
        .save_dir
        .clone()
        .unwrap_or_else(d2_client::app::save::default_save_dir);
    let front: Shared = Rc::new(RefCell::new(None));
    // The screens' cells, kept across games: online play's session and
    // screens outlive each game (back to the lobby on the same session).
    #[cfg_attr(not(feature = "online"), allow(unused_mut))]
    let mut start = StartHandles::default();
    #[cfg(feature = "online")]
    {
        start.server = o.server.clone();
    }
    // The last game was online: the front end opens at the lobby or the
    // login, with why it ended (`OnlineEnd`).
    let last_online = Rc::new(std::cell::Cell::new(false));
    #[cfg(feature = "online")]
    let online_end = d2_client::app::online::OnlineEnd::default();
    let starter = {
        let (front, saves, live) = (front.clone(), saves.clone(), live.clone());
        let (start, last_online, online) = (start.clone(), last_online.clone(), o.online);
        #[cfg(feature = "online")]
        let online_end = online_end.clone();
        FrontStarter(Box::new(move |after_game| {
            use d2_data::bin::TableFiles;
            d2_client::launch::note("front end (main menu)");
            // Music on: the jukebox plays in the menus.
            let music = d2_client::audio::environment::frontend_stream::FrontMusic::new(
                live.archives.source(),
                live.archives.lod(),
                true,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(1, |t| t.subsec_nanos()),
            );
            let mut art = FrontArt::new(live.archives.source());
            if let Ok(t) = d2_client::app::strings::TableStrings::load(
                live.archives.as_ref(),
                d2_client::app::strings::LANG,
            ) {
                art = art.with_strings(move |id| {
                    u16::try_from(id).map(|i| t.by_id(i)).unwrap_or_default()
                });
            }
            if let Ok(app) = d2_client::app::save::appearance_tables(&live.tables.fixed) {
                art = art.with_dolls(d2_client::ui::front_end::doll::DollTables::new(app));
            }
            // After a game: the main menu, or after an online game the
            // lobby (the login when the session is gone). `play --online`
            // opens at the login.
            let entry = match (after_game, last_online.replace(false)) {
                #[cfg(feature = "online")]
                (true, true) => {
                    d2_client::app::front_start::after_online_game(&start, online_end.take())
                }
                (true, _) => Entry::MainMenu,
                (false, _) if online => Entry::Online,
                (false, _) => Entry::First,
            };
            let handles = start.next_visit();
            let host = front_host_with(&saves, Some(art), live.archives.lod(), entry, &handles);
            let host = host.with_music(music);
            *front.borrow_mut() = Some((handles, host.outcome_handle()));
            host
        }))
    };
    // The game's leave handles (its save, the dev tools' `load`).
    let handles: Rc<RefCell<Option<d2_client::app::play::PlayHandles>>> = Rc::default();
    let load: Rc<RefCell<Option<DevLoad>>> = Rc::default();
    let opts = Rc::new(o);
    let game = {
        let (front, saves, handles, load, o) = (
            front.clone(),
            saves.clone(),
            handles.clone(),
            load.clone(),
            opts.clone(),
        );
        let (last_online, live) = (last_online.clone(), live.clone());
        GameStarter(Box::new(move |world| {
            let next = load.borrow_mut().take();
            // The lobby joined a server game: it starts over the network,
            // without blocking the frame (the join's answer installs it).
            let online_load = front.borrow().as_ref().and_then(|(h, out)| {
                match out.lock().ok().and_then(|o| *o) {
                    Some(Outcome::GameLoad(g)) if g.online && next.is_none() => Some(h.clone()),
                    _ => None,
                }
            });
            if let Some(h) = online_load {
                last_online.set(true);
                return start_online_game(world, &live, &h, &handles);
            }
            let (choice, menu_difficulty) = match &next {
                Some(_) => (None, None),
                None => {
                    let picked = front.borrow().as_ref().and_then(|(h, out)| {
                        match out.lock().ok().and_then(|o| *o) {
                            Some(Outcome::GameLoad(g)) => {
                                StartChoice::resolve(g, h, &saves).map(|c| (c, g.difficulty))
                            }
                            _ => None,
                        }
                    });
                    let (c, d) = picked.context("the front end chose no character")?;
                    (Some(c), d)
                }
            };
            let config = game_config(
                &o,
                data.clone(),
                dir.clone(),
                &origin,
                menu_difficulty,
                choice,
                next.as_ref(),
            )?;
            let ps = d2_client::app::play::play_settings(&config)?;
            world.insert_resource(d2_client::app::ui::LoadBakedAnimations);
            let h = d2_client::app::play::install_play(world, config, ps)?;
            *handles.borrow_mut() = Some(h);
            Ok(())
        }))
    };
    let ender = {
        let (handles, load) = (handles.clone(), load.clone());
        GameEnder(Box::new(move |world| {
            let Some(h) = handles.borrow_mut().take() else {
                return Ok(false);
            };
            let save = h.saver.as_ref().map(|s| s.path().to_path_buf());
            let end = d2_client::app::play::play_end(world, &h, bevy::app::AppExit::Success)?;
            match (&end.relaunch, save) {
                (Some(slot), Some(save)) => {
                    *load.borrow_mut() = Some(DevLoad::read(slot, save)?);
                    Ok(true)
                }
                (Some(_), None) => {
                    eprintln!("play: dev load: this game has no save file");
                    Ok(false)
                }
                _ => Ok(false),
            }
        }))
    };
    let cfg_dir = d2_client::app::config::config_dir(&saves);
    let settings = d2_client::app::config::load_settings(&cfg_dir)?;
    // The frame of every game of the session: `play --res`, else the
    // settings' Resolution row (a change on the row applies at the next
    // start).
    let frame = d2_client::app::config::set_play_frame(&settings)?;
    println!("play: frame {} x {}", frame.width, frame.height);
    let mut app = bevy::app::App::new();
    // The window the player chose; the front end and the game present their
    // pictures in it.
    let window = d2_client::app::config::window_for(&settings);
    let plugins = bevy::app::PluginGroup::set(
        d2_client::mute::plugins(bevy::DefaultPlugins),
        bevy::window::WindowPlugin {
            primary_window: Some(window),
            // The system pointer starts hidden: the front end draws the
            // game's cursor into its picture, and the game shows its own
            // (`app::session` keeps the pointer hidden on every screen).
            primary_cursor_options: Some(d2_client::app::session::hidden_cursor()),
            ..Default::default()
        },
    );
    let dev = d2_client::app::dev::enabled(
        opts.no_dev,
        d2_client::launch::env(d2_client::app::dev::ENV).as_deref(),
        false,
    );
    // game.log, and with the dev tools the log ring the bug report reads.
    app.add_plugins(plugins.set(d2_client::launch::log_plugin(dev)));
    d2_client::app::perf::add(&mut app);
    d2_client::app::play::register_play(&mut app, true);
    session::add_session(&mut app);
    d2_client::app::config::add_window_choice(&mut app, &settings, cfg_dir.clone());
    // The settings in force from the first screen on: the main menu's
    // Options screen shows, saves and applies them (`config::front_settings`),
    // and the menus' frame cap reads them. A game start reloads them.
    let bindings = d2_client::app::config::load_controls(&d2_client::app::config::controls_dir(
        d2_client::ui::front_end::screens::controls::config_path().as_deref(),
        &cfg_dir,
    ))?;
    app.insert_resource(d2_client::app::config::ConfigRes {
        dir: cfg_dir,
        settings,
        bindings,
        sound_device: !opts.no_sound,
    });
    #[cfg(feature = "online")]
    app.insert_resource(online_end);
    app.insert_non_send(game).insert_non_send(ender);
    add_front_session(&mut app, starter);
    match app.run() {
        bevy::app::AppExit::Success => Ok(()),
        bevy::app::AppExit::Error(code) => bail!("play exited with code {code}"),
    }
}

/// The online game the lobby joined, on the session the screens opened
/// (`app::online::start_online`).
#[cfg(feature = "online")]
fn start_online_game(
    world: &mut bevy::ecs::world::World,
    live: &std::sync::Arc<d2_client::app::single_player::LiveData>,
    h: &d2_client::app::front_start::StartHandles,
    handles: &std::rc::Rc<std::cell::RefCell<Option<d2_client::app::play::PlayHandles>>>,
) -> Result<()> {
    use d2_client::app::online::{start_online, OnlineJoin};
    let join = h
        .online
        .borrow()
        .as_ref()
        .and_then(|o| OnlineJoin::take(&mut o.borrow_mut()))
        .context("the lobby made no join")?;
    world.insert_resource(d2_client::app::ui::LoadBakedAnimations);
    start_online(world, live, &h.session, join)?;
    *handles.borrow_mut() = Some(d2_client::app::play::PlayHandles {
        saver: None,
        dev_relaunch: Default::default(),
    });
    Ok(())
}

#[cfg(not(feature = "online"))]
fn start_online_game(
    _: &mut bevy::ecs::world::World,
    _: &std::sync::Arc<d2_client::app::single_player::LiveData>,
    _: &d2_client::app::front_start::StartHandles,
    _: &std::rc::Rc<std::cell::RefCell<Option<d2_client::app::play::PlayHandles>>>,
) -> Result<()> {
    bail!("this build has no online play")
}

/// `front-music --out FILE [--game-dir DIR]`: the front-end jukebox's
/// first pick, run
/// headless; writes the `front-music-dump-1` records (header, then one
/// `start` per music voice start) for `tools/audio-diff/frontend_music.py`.
fn front_music(o: Options) -> Result<()> {
    use d2_client::audio::environment::frontend_stream::FrontMusic;
    use d2_data::bin::TableFiles;
    use sha2::{Digest, Sha256};
    let out = o.out.clone().context("front-music needs --out FILE")?;
    let (data, _dir, _origin) = select_data(&o)?;
    let d2_client::app::single_player::GameData::Live(d) = &data;
    let mut m = FrontMusic::new(d.archives.source(), d.archives.lod(), true, 1);
    m.service();
    let mut text = String::from("{\"format\": \"front-music-dump-1\"}\n");
    for e in &m.events {
        let sha: String = Sha256::digest(&e.first_block)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        text += &format!(
            "{{\"k\": \"start\", \"track\": \"{}\", \"volume\": {}, \"device_volume\": {}, \"first_block_len\": {}, \"first_block_sha256\": \"{sha}\"}}\n",
            e.track.replace('\\', "/"),
            e.volume,
            e.device_volume,
            e.first_block.len()
        );
    }
    std::fs::write(&out, text).with_context(|| format!("writing {}", out.display()))?;
    println!(
        "front-music: {} start(s) -> {}",
        m.events.len(),
        out.display()
    );
    Ok(())
}

/// One game's config: the front end's `choice` (with its menu
/// difficulty), a dev `load`, or the command line's start.
fn game_config(
    o: &Options,
    data: d2_client::app::single_player::GameData,
    dir: Option<PathBuf>,
    origin: &str,
    menu_difficulty: Option<u8>,
    choice: Option<d2_client::app::front_start::StartChoice>,
    load: Option<&DevLoad>,
) -> Result<d2_client::app::play::PlayConfig> {
    use d2_client::app::{play, single_player};
    let single_player::GameData::Live(d) = &data;
    println!(
        "play: game data from {origin} ({} levels, {} objects, waypoint object class {}; level files: {} DS1, {} lvlsub DS1, {} DT1)",
        d.waypoints.levels.len(),
        d.waypoints.objects.len(),
        d.waypoints.object_class,
        d.files.ds1.0.len(),
        d.files.subs.0.len(),
        d.files.dt1.0.len()
    );
    // Before the window opens: a bad folder or a name taken stops here.
    let start = d2_client::app::play_start::resolve(
        &d2_client::app::play_start::CliStart {
            save: load.map(|l| l.save.clone()).or_else(|| o.save.clone()),
            new: if load.is_some() { None } else { o.new.clone() },
            save_dir: o.save_dir.clone(),
            difficulty: load.map_or(o.difficulty, |l| l.difficulty),
            hardcore: o.hardcore,
        },
        &data,
        dir.as_deref(),
        menu_difficulty,
        choice.as_ref(),
    )?;
    if choice.is_some() || o.save.is_some() || o.new.is_some() {
        println!("play: {}", start.origin);
    }
    println!("play: difficulty {}", start.difficulty);
    // The front end's game draws its seed from the clock;
    // `play` without it keeps the fixed default (dev runs, draw dumps).
    let drawn = match &choice {
        Some(_) => single_player::clock_map_seed(),
        None => single_player::DEFAULT_SEED,
    };
    let seed =
        single_player::game_seed_or(&start.character, load.map(|l| l.seed).or(o.seed), drawn);
    d2_client::launch::note(format!(
        "game start: {}, difficulty {}, seed {seed}, save {:?}",
        start.origin, start.difficulty, start.save_path
    ));
    Ok(play::PlayConfig {
        data,
        seed,
        character: start.character,
        exit_after: o.frames,
        save_path: start.save_path,
        start_flags: start.start_flags,
        hardcore: start.hardcore,
        dump: o
            .dump_draws
            .clone()
            .map(|dir| d2_client::facts::export::DumpRequest {
                dir,
                at_ticks: o.at_tick.clone().unwrap_or_else(|| vec![1]),
                image: o.dump_image,
                native: o.dump_native.clone(),
                command: std::env::args().collect::<Vec<_>>().join(" "),
            }),
        input: o.input.clone(),
        pokes: o.pokes.clone(),
        sends: o.sends.clone(),
        sound_log: o.sound_log.clone(),
        no_sound: o.no_sound,
        registry: match &o.registry {
            Some(p) => Some(
                d2_client::app::registry::HostRegistry::parse(
                    &std::fs::read_to_string(p)
                        .with_context(|| format!("--registry {}", p.display()))?,
                )
                .map_err(|e| anyhow::anyhow!("--registry {}: {e}", p.display()))?,
            ),
            None => None,
        },
        frame_schedule: match &o.frame_schedule {
            Some(p) => Some(
                d2_client::world_view::present::FrameSchedule::parse(
                    &std::fs::read_to_string(p)
                        .with_context(|| format!("--frame-schedule {}", p.display()))?,
                )
                .map_err(|e| anyhow::anyhow!("--frame-schedule {}: {e}", p.display()))?,
            ),
            None => None,
        },
        audio_dump: o.audio_dump.clone().map(|p| (p, o.audio_ticks)),
        device_ends: match &o.device_ends {
            Some(p) => Some(
                d2_client::app::sound::DeviceEnds::parse(&std::fs::read_to_string(p)?)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?,
            ),
            None => None,
        },
        async_collects: match &o.async_collects {
            Some(p) => Some(
                d2_client::app::sound::AsyncCollects::parse(&std::fs::read_to_string(p)?)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?,
            ),
            None => None,
        },
        loading_dump: o.dump_loading.clone(),
        dev: d2_client::app::dev::enabled(
            o.no_dev,
            d2_client::launch::env(d2_client::app::dev::ENV).as_deref(),
            false,
        ),
        difficulty: start.difficulty,
        dev_start: load.map(|l| l.slot.clone()).or_else(|| o.dev_start.clone()),
        offscreen: o
            .offscreen
            .then(|| d2_client::world_view::shots::Offscreen {
                size: o.size.unwrap_or(d2_client::world_view::shots::DEFAULT_SIZE),
                shots: o
                    .shot_dir
                    .clone()
                    .map(|dir| d2_client::world_view::shots::ShotPlan {
                        dir,
                        at: o.shot_at.clone(),
                        every: o.shot_every,
                        delay: o
                            .shot_phase
                            .as_deref()
                            .zip(o.display_clock.as_ref())
                            .and_then(|(p, c)| c.phase_frame(p).ok())
                            .unwrap_or(0),
                    }),
                display_clock: o.display_clock.clone(),
            }),
        control: o.control,
    })
}

/// `audio-mix VOICES OUT --last-tick N`:
/// a voice list (`audio-voices-1`) mixed through D2RUST's mixer, one `mix`
/// line per tick (the `d2rust-audio-dump-1` format, header first).
fn audio_mix(args: &[String]) -> Result<()> {
    use d2_client::app::audio_dump::{header_line, mix_list, parse_voice_list, record_line};
    let [voices, out, flag, last] = args else {
        bail!("usage: d2-client audio-mix VOICES OUT --last-tick N");
    };
    if flag != "--last-tick" {
        bail!("usage: d2-client audio-mix VOICES OUT --last-tick N");
    }
    let last: u32 = last.parse().context("--last-tick")?;
    let path = std::path::Path::new(voices);
    let text = std::fs::read_to_string(path).with_context(|| voices.to_string())?;
    let base = path.parent().unwrap_or(std::path::Path::new("."));
    let list = parse_voice_list(&text, base).map_err(|e| anyhow::anyhow!("{voices}: {e}"))?;
    let recs = mix_list(&list, last).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut text = header_line() + "\n";
    for r in &recs {
        text += &record_line(r, None);
        text.push('\n');
    }
    std::fs::write(out, text).with_context(|| out.to_string())?;
    println!(
        "audio-mix: {} voices, {} ticks -> {out}",
        list.len(),
        recs.len()
    );
    Ok(())
}

/// `facts-compare BASE D2RUST [--ignore COL,...] [--skip-weather]`: the first
/// difference between two facts dumps; exit 0 match, 1 diverged, 2 partial, 3 error.
fn facts_compare(args: &[String]) -> i32 {
    let mut dirs = Vec::new();
    let mut ignore = Vec::new();
    let mut skip_weather = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--ignore" => match it.next() {
                Some(v) => ignore.extend(v.split(',').map(|c| c.trim().to_owned())),
                None => {
                    eprintln!("facts-compare: --ignore needs a value");
                    return 3;
                }
            },
            "--skip-weather" => skip_weather = true,
            _ => dirs.push(PathBuf::from(a)),
        }
    }
    let [original, d2rust] = &dirs[..] else {
        eprintln!(
            "usage: d2-client facts-compare ORIGINAL_DIR D2RUST_DIR [--ignore COL,...] [--skip-weather]"
        );
        return 3;
    };
    match d2_client::facts::compare::compare_dirs(original, d2rust, &ignore, skip_weather) {
        Ok(outcome) => {
            println!("{outcome}");
            outcome.exit_code()
        }
        Err(e) => {
            eprintln!("facts-compare: ERROR: {e}");
            3
        }
    }
}

/// `state-dump ...`: the game-state
/// snapshots of a headless `play` run.
fn state_dump(args: &[String]) -> Result<()> {
    use d2_client::app::state_dump;
    let a = state_dump::parse_args(args)?;
    let command = std::env::args().collect::<Vec<_>>().join(" ");
    let r = state_dump::run(&a, &command)?;
    println!(
        "state-dump: {} ticks, {} snapshots -> {}",
        r.ticks,
        r.snaps,
        a.out.display()
    );
    Ok(())
}

fn main() -> std::process::ExitCode {
    use d2_client::launch;
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `--version`: one line on stdout for the launcher, nothing else runs.
    if matches!(
        args.first().map(String::as_str),
        Some("--version" | "-V" | "version")
    ) {
        println!("D2RUST {}", launch::VERSION);
        return std::process::ExitCode::SUCCESS;
    }
    if let Some(path) = launch::take_error_file(&mut args) {
        launch::set_error_file(path);
    }
    // `--mute` / `D2RUST_MUTE=1` / an offscreen run: no sound device.
    let muted = d2_client::mute::init(&mut args);
    // The player's D2RUST folder (saves, settings, controls, logs), moving
    // an old `Documents\d2rs` into it on the first start; then this run's
    // game.log in it.
    let (user, moved) = d2_client::user_dir::init();
    let log = launch::open_log();
    launch::install_crash_log();
    d2_client::say!("D2RUST {} ({})", launch::VERSION, user.display());
    if let Some(log) = &log {
        launch::note(format!("log: {}", log.display()));
    }
    if muted {
        launch::note("sound: muted (no output device)".to_string());
    }
    match moved {
        d2_client::user_dir::Migration::None => {}
        m => d2_client::say!("user folder: {m:?}"),
    }
    launch::note(format!("user folder: {}", user.display()));
    // tools/perf: D2_PERF_OUT turns the timing on (server ticks, frames).
    d2_client::app::perf::enable_from_env();
    let result = run(args);
    // The coverage hook (a no-op in D2RUST).
    d2_sim::debug::coverage::flush();
    d2_client::app::perf::write_report();
    match &result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            // The message on stderr too, as `main` returning the error did.
            d2_client::say_err!("Error: {e:?}");
            launch::write_error(e);
            pause_if_console();
            std::process::ExitCode::from(launch::exit_code(e))
        }
    }
}

/// A double-clicked exe's console closes on exit: on Windows, keep it
/// open on an error until Enter, so the message can be read.
fn pause_if_console() {
    use std::io::IsTerminal;
    if cfg!(windows) && std::io::stdin().is_terminal() {
        eprintln!("press Enter to close");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}

fn run(args: Vec<String>) -> Result<()> {
    use d2_client::launch;
    launch::note(format!("command: {}", args.join(" ")));
    // The install folder (`launch::game_dir`): `--game-dir`, $D2_GAME_DIR,
    // then the exe's or the current folder when it holds d2data.mpq. Set
    // as $D2_GAME_DIR before any thread starts, for every reader of it.
    let arg_dir = args
        .windows(2)
        .find(|w| w[0] == "--game-dir")
        .map(|w| PathBuf::from(&w[1]));
    let env_dir = std::env::var_os("D2_GAME_DIR").map(PathBuf::from);
    if let Some(dir) = launch::game_dir(arg_dir.as_deref(), env_dir, &launch::candidates()) {
        d2_client::say!("game folder: {}", dir.display());
        launch::note(format!("game folder: {}", dir.display()));
        std::env::set_var("D2_GAME_DIR", &dir);
    }
    // The bake folder (dev/test override; the launcher sets D2_ANIM_DIR).
    if let Some(dir) = args.windows(2).find(|w| w[0] == "--anim-dir") {
        d2_client::world_view::unit_rules::animation_baked::set_dir(PathBuf::from(&dir[1]));
    }
    match args.first().map(String::as_str) {
        Some("animation-proof") => d2_client::world_view::unit_rules::animation_proof::run_cli(&args[1..]).map_err(anyhow::Error::msg),
        Some("bake-anims") => d2_client::world_view::unit_rules::animation_bake::run_cli(&args[1..]).map_err(anyhow::Error::msg),
        Some("animation-build") => d2_client::world_view::unit_rules::animation_build::run_cli(&args[1..]).map_err(anyhow::Error::msg),
        Some("facts-compare") => std::process::exit(facts_compare(&args[1..])),
        Some("perf-summary") => std::process::exit(d2_client::app::perf_summary::run_cli(&args[1..])),
        Some("audio-mix") => audio_mix(&args[1..]),
        Some("front-music") => front_music(parse_options(&args[1..])?),
        Some("state-dump") => state_dump(&args[1..]),
        Some("soak") => {
            let a = d2_client::app::soak::parse_args(&args[1..])?;
            std::process::exit(d2_client::app::soak::run(&a)?)
        }
        Some("explore") => d2_client::app::explore::main(&args[1..]),
        Some("autoplay-host") => d2_client::app::autoplay_host::serve(&args[1..]),
        Some("sweep") => {
            let code = d2_client::app::sweep::main(&args[1..]);
            d2_client::app::perf::write_report();
            std::process::exit(i32::from(code))
        }
        Some("repro") => {
            let code = d2_client::app::repro::main(&args[1..]);
            // `exit` skips the report `main` writes on return: write it here,
            // so a scenario run under D2_PERF_OUT has its timing too.
            d2_client::app::perf::write_report();
            std::process::exit(i32::from(code))
        }
        Some("map") => std::process::exit(i32::from(d2_client::app::game_map::main(&args[1..]))),
        Some("cpu-render") => cpu_render(parse_options(&args[1..])?),
        Some("verify") => verify(parse_options(&args[1..])?),
        Some("play") | None => play(parse_options(args.get(1..).unwrap_or(&[]))?),
        Some("view") => view(parse_options(&args[1..])?),
        // Proves the crash log (`launch`, windows-build.yml smoke step).
        Some("crash-test") => panic!("crash-test: a deliberate panic to check D2RUST-crash.log"),
        _ => Err(launch::Usage("usage: D2RUST-Game [--version] [--error-file FILE] [--mute | --sound] [view|verify|cpu-render|map|play|facts-compare|perf-summary|audio-mix|front-music|state-dump|soak|autoplay-host|explore|repro|sweep] [--ds1 PATH] [--wall-base N] [--view L,T,W,H] [--out PATH] [--case NAME] [--cases DIR] [--perturb N] [--seed N] [--frames N] [--difficulty normal|nightmare|hell] [--save FILE.d2s | --new CLASS NAME [--save-dir DIR]] [--native DIR] [--source native|mpq] [--game-dir DIR] [--dump-draws DIR [--at-tick N[,M...]] [--dump-image]] [--sound-log FILE] [--audio-dump FILE [--audio-ticks N]] [--res 800x600|640x480|1068x600] [--input SCRIPT] [--poke \"F DIRECTIVE ARGS\"]... [--poke-file FILE] [--no-dev] [--dev-start SLOT]".into()).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_parsing() {
        let v = parse_view("-10, 20,300,400").unwrap();
        assert_eq!((v.left, v.top, v.width, v.height), (-10, 20, 300, 400));
        assert!(parse_view("1,2,3").is_err());
        assert!(parse_view("1,2,-3,4").is_err());
    }

    #[test]
    fn stems() {
        assert_eq!(file_stem(DEFAULT_DS1), "townN1");
    }

    #[test]
    fn new_takes_a_class_and_a_name() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let o = parse_options(&args(&["--new", "amazon", "Test", "--frames", "3"])).unwrap();
        assert_eq!(o.new, Some(("amazon".to_owned(), "Test".to_owned())));
        assert_eq!(o.frames, Some(3));
        assert!(parse_options(&args(&["--new", "amazon"])).is_err());
        assert!(parse_options(&args(&["--new"])).is_err());
        assert!(parse_options(&args(&["--new", "0", "A", "--save", "x.d2s"])).is_err());
    }

    #[test]
    fn offscreen_runs_need_a_character_an_end_and_a_shot_folder() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let new = ["--new", "sorceress", "Shot"];
        let with = |extra: &[&str]| parse_options(&args(&[new.as_slice(), extra].concat()));
        let o = with(&[
            "--offscreen",
            "--size",
            "1280x720",
            "--shot-at",
            "20,10",
            "--shot-dir",
            "s",
        ])
        .unwrap();
        assert!(o.offscreen);
        assert_eq!(o.size, Some((1280, 720)));
        assert_eq!(o.shot_at, [10, 20]);
        assert_eq!(o.shot_dir, Some(PathBuf::from("s")));
        let o = with(&[
            "--offscreen",
            "--frames",
            "300",
            "--shot-every",
            "30",
            "--shot-dir",
            "s",
        ])
        .unwrap();
        assert_eq!(o.shot_every, Some(30));
        assert!(with(&["--offscreen", "--frames", "10"]).is_ok());
        let o = with(&[
            "--offscreen",
            "--frames",
            "9",
            "--display-clock",
            "fixed:60",
        ])
        .unwrap();
        assert_eq!(
            o.display_clock,
            Some(d2_client::world_view::display_clock::DisplayClock::Fixed { fps: 60 })
        );
        assert!(with(&["--frames", "9", "--display-clock", "fixed:60"]).is_err());
        // A shot phase is one of the clock's phases, for planned shots.
        let phased = |p: &str, clock: &str| {
            with(&[
                "--offscreen",
                "--display-clock",
                clock,
                "--shot-at",
                "40",
                "--shot-dir",
                "s",
                "--shot-phase",
                p,
            ])
        };
        assert!(phased("0.5", "phases:0,0.5").is_ok());
        assert!(phased("0.25", "phases:0,0.5").is_err());
        assert!(phased("0.5", "fixed:60").is_err());
        assert!(with(&["--offscreen", "--frames", "9", "--display-clock", "60"]).is_err());
        // Shot flags without --offscreen, shots without a folder and a
        // folder without shots are refused.
        assert!(with(&["--shot-at", "5", "--shot-dir", "s", "--frames", "9"]).is_err());
        assert!(with(&["--offscreen", "--shot-at", "5"]).is_err());
        assert!(with(&["--offscreen", "--frames", "9", "--shot-dir", "s"]).is_err());
        // No end: neither --frames nor a --shot-at list that finishes.
        assert!(with(&["--offscreen"]).is_err());
        assert!(with(&["--offscreen", "--shot-every", "5", "--shot-dir", "s"]).is_err());
        assert!(with(&["--offscreen", "--shot-every", "0", "--frames", "9"]).is_err());
        // An offscreen run plays a character, so it skips the front end.
        assert!(parse_options(&args(&["--offscreen", "--frames", "9"])).is_err());
    }

    #[test]
    fn control_runs_need_a_character_and_end_offscreen_runs() {
        use d2_client::app::control::Transport;
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let new = ["--new", "sorceress", "Ctl"];
        let with = |extra: &[&str]| parse_options(&args(&[new.as_slice(), extra].concat()));
        let o = with(&["--control", "stdio"]).unwrap();
        assert_eq!(o.control, Some(Transport::Stdio));
        // The channel ends an offscreen run: no --frames needed.
        let o = with(&["--offscreen", "--control", "pipe"]).unwrap();
        assert_eq!(o.control, Some(Transport::Pipe));
        assert!(with(&["--control", "tcp"]).is_err());
        assert!(with(&["--control"]).is_err());
        assert!(parse_options(&args(&["--control", "stdio"])).is_err());
    }

    #[test]
    fn input_takes_a_script() {
        use d2_client::world_view::input_script::Step;
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let o = parse_options(&args(&["--input", "wait 3; click 600 300"])).unwrap();
        assert_eq!(o.input.as_deref().map(<[Step]>::len), Some(2));
        assert!(parse_options(&args(&["--input", "fly 1 2"])).is_err());
        assert!(parse_options(&args(&["--input"])).is_err());
        // Points are checked against the `--res` frame, wherever the flag
        // stands: x 1040 fits 1068 × 600, not 800 × 600.
        let wide = ["--input", "click 1040 385", "--res", "1068x600"];
        assert!(parse_options(&args(&wide)).is_ok());
        let narrow = ["--res", "800x600", "--input", "click 1040 385"];
        assert!(parse_options(&args(&narrow)).is_err());
        assert!(parse_options(&args(&["--input", "click 1067 599"])).is_ok());
        assert!(parse_options(&args(&["--input", "click 1068 0"])).is_err());
    }

    #[test]
    fn res_takes_the_three_frames() {
        use d2_client::rules::camera::FrameSize;
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_options(&[]).unwrap().res, None);
        let o = parse_options(&args(&["--res", "640x480"])).unwrap();
        assert_eq!(o.res, Some(FrameSize::LOW));
        let o = parse_options(&args(&["--res", "800x600"])).unwrap();
        assert_eq!(o.res, Some(FrameSize::D2RUST));
        let o = parse_options(&args(&["--res", "1068x600"])).unwrap();
        assert_eq!(o.res, Some(FrameSize::WIDE));
        assert!(parse_options(&args(&["--res", "1024x768"])).is_err());
        assert!(parse_options(&args(&["--res"])).is_err());
    }

    #[test]
    fn dump_draws_takes_a_dir_and_a_tick() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let o = parse_options(&args(&["--dump-draws", "d", "--at-tick", "40"])).unwrap();
        assert_eq!(
            (o.dump_draws, o.at_tick),
            (Some(PathBuf::from("d")), Some(vec![40]))
        );
        assert!(parse_options(&args(&["--at-tick", "40"])).is_err());
        assert!(parse_options(&args(&["--dump-draws"])).is_err());
        assert!(parse_options(&args(&["--dump-image"])).is_err());
        let o = parse_options(&args(&[
            "--dump-draws",
            "d",
            "--at-tick",
            "2,40,73",
            "--dump-image",
        ]))
        .unwrap();
        assert_eq!((o.at_tick, o.dump_image), (Some(vec![2, 40, 73]), true));
        assert!(parse_options(&args(&["--dump-draws", "d", "--at-tick", "40,40"])).is_err());
        assert!(parse_options(&args(&["--dump-draws", "d", "--at-tick", "40,"])).is_err());
        let o = parse_options(&args(&["--sound-log", "s.tsv"])).unwrap();
        assert_eq!(o.sound_log, Some(PathBuf::from("s.tsv")));
        let o = parse_options(&args(&["--dump-draws", "d", "--no-sound"])).unwrap();
        assert!(o.no_sound);
        let o = parse_options(&args(&["--registry", "r.tsv"])).unwrap();
        assert_eq!(o.registry, Some(PathBuf::from("r.tsv")));
        assert!(parse_options(&args(&["--registry"])).is_err());
        assert!(parse_options(&args(&["--frame-schedule", "f.tsv"])).is_err());
        // an audio check follows the schedule's drawn frames
        let o = parse_options(&args(&[
            "--frame-schedule",
            "f.tsv",
            "--audio-dump",
            "a.jsonl",
        ]));
        assert_eq!(o.unwrap().frame_schedule, Some(PathBuf::from("f.tsv")));
    }

    #[test]
    fn facts_compare_usage_errors_exit_3() {
        assert_eq!(facts_compare(&[]), 3);
        assert_eq!(facts_compare(&["a".into(), "--ignore".into()]), 3);
        assert_eq!(
            facts_compare(&["/no/such/a".into(), "/no/such/b".into()]),
            3
        );
    }

    #[test]
    fn map_flags_select_the_single_map_verify() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!parse_options(&[]).unwrap().map_flags);
        let o = parse_options(&args(&["--perturb", "3", "--case", "a"])).unwrap();
        assert!(!o.map_flags);
        assert_eq!((o.perturb, o.cases), (3, vec!["a".to_owned()]));
        for flag in [
            ["--ds1", "x.ds1"],
            ["--wall-base", "1"],
            ["--view", "0,0,2,2"],
            ["--out", "d"],
        ] {
            assert!(parse_options(&args(&flag)).unwrap().map_flags, "{flag:?}");
        }
    }
}
