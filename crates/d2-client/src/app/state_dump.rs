//! `d2-client state-dump`: the game `d2-client play --save X --seed N`
//! builds, run headless (no window, no Bevy) on a stepping clock, with
//! one `state-1` snapshot (`d2_sim::debug::state`) written after every
//! server tick that ran (or every n-th frame).
//!
//! The run: the game is built as `play` builds it
//! ([`super::play_start::resolve`] for `--save` / `--difficulty`,
//! [`single_player::game_seed`] for `--seed`,
//! [`single_player::start_with`], the hardcore switch), the client is the
//! plain [`Bridge`] (`play`'s link handler without the Bevy app) with the
//! bridge-level client data `play` installs (client DRLG source, level,
//! object, skill and unit rows). The bridge sends the 0x67 create request
//! before the first pump and answers 0x02 with 0x6B on its own, as in
//! `play`. Each step advances the clock to the next tick and runs one bridge frame
//! (pump: drain → tick → flush; receive; the model's own answers). After
//! a frame whose pump ticked, the snapshot (the state after
//! tick N, `f` = `Game::frame` = N) is taken on the server thread
//! through [`ThreadLink::with`], reading only.
//!
//! Pokes: each `--poke "<f> <directive>
//! <args>..."` runs through the bridge ([`Bridge::poke`], on the server
//! thread) right after the snapshot of frame f − 1, before frame f's
//! drain; its result is a `poke` line between the two snapshots (the
//! record `poke.py` writes) and a line on stderr.
//!
//! Input: `--input SCRIPT`,
//! the shared frame-anchored form (`frame F; click X Y; hold X Y N; move X
//! Y`), runs at the same point as the pokes, after them: each due step's
//! pointer events go through the bridge's world-click dispatcher
//! ([`Headless`]); its log lines go to stderr and the footer notes.
//!
//! Sends (the scenario `at … send` step): each `--send
//! "<f> <Name> <field>=<value>..."` or `--send "<f> hex <byte>..."` is
//! injected through the bridge ([`Bridge::inject`]: references resolved
//! on the server game, bytes handed to the transport send after the
//! duplicate filter, as a scenario send) at the same
//! point, after the pokes and the input, in command-line order; each
//! result is a `send` line (bytes or unresolved), a line on stderr and a
//! footer note.
//!
//! Sender clock: `--send-clock
//! FILE` (`# send-clock 1`, then `frame\tnow` rows, a reference sender's
//! `GetTickCount` per frame) is handed to the host before the
//! first frame; the c2s duplicate filter then compares against the
//! listed time of the latest listed frame at or before the current one.
//!
//! Output: header, snaps (and poke / send lines), footer. Two runs with the same arguments
//! write the same bytes but for the header's `date`. Not game logic: the
//! clock and the date are this binary's (the determinism rule binds
//! `d2-sim`, which only reads here).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use d2_server::seams::Clock;
use d2_sim::debug::state;

use super::play_start::{self, CliStart};
use super::poke::{self as pokes, Entry, When};
use super::send::{self as sends, SendEntry};
use super::server_thread::ThreadLink;
use super::single_player::{self, Character, GameData, Link};
use crate::bridge::output::{Consumer, Output};
use crate::bridge::predict::PredictLink;
use crate::bridge::state::StateSource;
use crate::bridge::world::GfxReady;
use crate::bridge::Bridge;
use crate::world_view::input_script::{self, Headless};
use d2_sim::poke::{Directive, GotoWalk, PokeOp, PokeResult};

/// Milliseconds in a table-unit tick (25 Hz); game clocks use `step_ms`.
pub const STEP_MS: u32 = 40;

/// The clock's value at the first pump (as the headless app tests).
pub const START_MS: u32 = 1000;

/// Steps without a tick after which the run stops (the server is stuck).
const MAX_IDLE_STEPS: u32 = 1000;

/// A host clock the dump steps by hand.
#[derive(Clone, Debug, Default)]
pub struct StepClock(pub Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// `state-dump`'s options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DumpArgs {
    /// `--save FILE.d2s`: the character (as `play --save`); none: `play`'s
    /// default character.
    pub save: Option<PathBuf>,
    /// `--seed N`: the fixed game seed (as `play --seed`).
    pub seed: Option<u32>,
    /// `--difficulty normal|nightmare|hell|0-2`.
    pub difficulty: u8,
    /// `--ticks T`: server ticks to run.
    pub ticks: u32,
    /// Simulation rate for this headless run (60 Hz by default).
    pub tick_rate: d2_sim::tick::TickRate,
    /// `--every n`: snapshot the frames that are multiples of n (default 1).
    pub every: u32,
    /// `--out FILE`.
    pub out: PathBuf,
    /// `--game-dir DIR`, else `$D2_GAME_DIR`.
    pub game_dir: Option<PathBuf>,
    /// `--date YYYY-MM-DD` for the header, else today (UTC).
    pub date: Option<String>,
    /// `--poke "<f> <directive> <args>..."` (repeatable): absolute frames.
    pub pokes: Vec<Entry>,
    /// `--input SCRIPT`: the shared frame-anchored input.
    pub input: Option<Vec<input_script::Step>>,
    /// `--send "<f> <Name|hex>..."` (repeatable): scripted C→S messages.
    pub sends: Vec<SendEntry>,
    /// `--no-own-c2s <id>[,<id>]`: C→S ids the bridge's own sends drop.
    pub no_own_c2s: Vec<u8>,
    /// `--packets FILE`: also record the packets.
    pub packets: Option<PathBuf>,
    /// `--rng FILE`: also record every RNG draw ([`super::rng_dump`];
    /// needs the `rng-trace` feature).
    pub rng: Option<PathBuf>,
    /// `--save-out FILE`: install the character writer of `play` with this
    /// path, so a `--send "<f> hex 69"` (Save and Exit) writes the `.d2s`
    /// there (the `save` channel of the scenario diff).
    pub save_out: Option<PathBuf>,
    /// `--client-out FILE`: also write the client model's units per tick
    /// ([`super::client_state`]).
    pub client_out: Option<PathBuf>,
    /// `--client-grid`: add the `fp` footprint window of each set S object
    /// to the client dump ([`super::client_state::Footprints`]).
    pub client_grid: bool,
    /// `--early-hex BYTES`: C->S messages sent right after the 0x67 create request,
    /// before the first frame (the client's own startup ping).
    pub early: Vec<Vec<u8>>,
    /// `--no-ping`: the client's own ping (C->S 0x6D every 125 frames) off, for
    /// a run that replays a reference run's pings with `--early-hex` / `--send`.
    pub no_ping: bool,
    /// `--send-clock FILE`: the sender's recorded clock per frame
    /// ([`read_send_clock`]).
    pub send_clock: Option<PathBuf>,
    /// `--player-seed FILE`: the local player's recorded unit seed per frame
    /// ([`read_player_seed`]).
    pub player_seed: Option<PathBuf>,
    /// `--gfx-ready FILE`: a reference client-state dump whose snapshots
    /// carry `gfx_ready` ([`read_gfx_ready`]).
    pub gfx_ready: Option<PathBuf>,
}

/// Parses the options after `state-dump`.
pub fn parse_args(args: &[String]) -> Result<DumpArgs> {
    let mut a = DumpArgs {
        save: None,
        seed: None,
        difficulty: 0,
        ticks: 0,
        tick_rate: d2_sim::tick::TickRate::GAME,
        every: 1,
        out: PathBuf::new(),
        game_dir: None,
        date: None,
        pokes: Vec::new(),
        input: None,
        sends: Vec::new(),
        no_own_c2s: Vec::new(),
        packets: None,
        rng: None,
        save_out: None,
        client_out: None,
        client_grid: false,
        early: Vec::new(),
        no_ping: false,
        send_clock: None,
        player_seed: None,
        gfx_ready: None,
    };
    let (mut ticks, mut out) = (None, None);
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--save" => a.save = Some(PathBuf::from(value()?)),
            "--seed" => a.seed = Some(value()?.parse().context("--seed")?),
            "--difficulty" => {
                let v = value()?;
                a.difficulty = single_player::parse_difficulty(v).with_context(|| {
                    format!("--difficulty {v}: use normal, nightmare, hell or 0-2")
                })?;
            }
            "--ticks" => ticks = Some(value()?.parse::<u32>().context("--ticks")?),
            "--tick-rate" => {
                let hz = value()?.parse::<u32>().context("--tick-rate")?;
                a.tick_rate = d2_sim::tick::TickRate::new(hz)
                    .context("--tick-rate must be between 25 and 1000 Hz")?;
            }
            "--every" => a.every = value()?.parse().context("--every")?,
            "--out" => out = Some(PathBuf::from(value()?)),
            "--game-dir" => a.game_dir = Some(PathBuf::from(value()?)),
            "--packets" => a.packets = Some(PathBuf::from(value()?)),
            "--rng" => a.rng = Some(PathBuf::from(value()?)),
            "--save-out" => a.save_out = Some(PathBuf::from(value()?)),
            "--client-out" => a.client_out = Some(PathBuf::from(value()?)),
            "--client-grid" => a.client_grid = true,
            "--no-ping" => a.no_ping = true,
            "--send-clock" => a.send_clock = Some(PathBuf::from(value()?)),
            "--player-seed" => a.player_seed = Some(PathBuf::from(value()?)),
            "--gfx-ready" => a.gfx_ready = Some(PathBuf::from(value()?)),
            "--early-hex" => {
                let v = value()?;
                let bytes = v
                    .split_ascii_whitespace()
                    .map(|t| u8::from_str_radix(t, 16))
                    .collect::<Result<Vec<u8>, _>>()
                    .with_context(|| format!("--early-hex {v}: hex bytes"))?;
                if bytes.is_empty() {
                    bail!("--early-hex: at least one byte");
                }
                a.early.push(bytes);
            }
            "--poke" => a
                .pokes
                .push(pokes::parse_poke_arg(value()?).map_err(anyhow::Error::msg)?),
            "--send" => a
                .sends
                .push(sends::parse_send_arg(value()?).map_err(anyhow::Error::msg)?),
            "--no-own-c2s" => {
                for part in value()?.split(',') {
                    let t = part.trim();
                    let id = match t.strip_prefix("0x") {
                        Some(h) => u8::from_str_radix(h, 16),
                        None => t.parse::<u8>(),
                    }
                    .with_context(|| format!("--no-own-c2s {t}: a message id (0-255, 0x hex)"))?;
                    if !a.no_own_c2s.contains(&id) {
                        a.no_own_c2s.push(id);
                    }
                }
            }
            "--input" => {
                let steps = input_script::parse(value()?)
                    .and_then(|s| Headless::new(s.clone()).map(|_| s))
                    .map_err(|e| anyhow::anyhow!("--input: {e}"))?;
                a.input = Some(steps);
            }
            "--date" => {
                let v = value()?.clone();
                if !is_date(&v) {
                    bail!("--date {v}: use YYYY-MM-DD");
                }
                a.date = Some(v);
            }
            other => bail!("state-dump: unknown option {other}"),
        }
    }
    a.ticks = ticks.context("state-dump needs --ticks T")?;
    a.out = out.context("state-dump needs --out FILE")?;
    if a.every == 0 {
        bail!("--every must be at least 1");
    }
    Ok(a)
}

/// Absolute tick boundaries avoid rounding drift at rates such as 60 Hz.
pub(super) fn step_ms(rate: d2_sim::tick::TickRate, step: u32) -> u32 {
    START_MS.wrapping_add(rate.ms_for_ticks(step) as u32)
}

/// Parses a send-clock file: a
/// `# send-clock 1` first line, then `frame<TAB>now` rows (decimal).
pub fn parse_send_clock(text: &str, rate: d2_sim::tick::TickRate) -> Result<BTreeMap<i32, u32>> {
    crate::recording::validate(
        crate::recording::text_rate(text).map_err(anyhow::Error::msg)?,
        rate,
    )
    .map_err(anyhow::Error::msg)?;
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("# send-clock 1") {
        bail!("send clock: the first line must be '# send-clock 1'");
    }
    let mut out = BTreeMap::new();
    for (n, line) in lines.enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let row = line
            .split_once('\t')
            .and_then(|(f, t)| Some((f.trim().parse().ok()?, t.trim().parse().ok()?)));
        let Some((frame, now)) = row else {
            bail!("send clock line {}: '{line}': use frame<TAB>now", n + 2);
        };
        out.insert(frame, now);
    }
    Ok(out)
}

/// The monster graphics-ready flags of a reference client-state dump (key
/// `gfx_ready`): tick → the flags at that tick's end, for each snapshot that
/// has them (`[[class, mask],...]`, bit m = mode m; `null` or absent: not read,
/// no entry). A host input: the dead draw's direction write-back depends on it.
pub fn parse_gfx_ready(
    text: &str,
    rate: d2_sim::tick::TickRate,
) -> Result<BTreeMap<u64, GfxReady>> {
    let header: serde_json::Value = serde_json::from_str(text.lines().next().unwrap_or(""))
        .context("recording header: expected JSON with tick_rate")?;
    let hz = header
        .get("tick_rate")
        .and_then(|n| n.as_u64())
        .and_then(|n| u32::try_from(n).ok());
    crate::recording::validate(hz, rate).map_err(anyhow::Error::msg)?;
    let mut out = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        if !line.contains("gfx_ready") {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(line).with_context(|| format!("line {}", n + 1))?;
        if v.get("k").and_then(|k| k.as_str()) != Some("snap") {
            continue;
        }
        let (Some(f), Some(pairs)) = (
            v.get("f").and_then(|f| f.as_u64()),
            v.get("gfx_ready").and_then(|g| g.as_array()),
        ) else {
            continue;
        };
        let mut r = GfxReady::default();
        for p in pairs {
            let pair = p.as_array().filter(|a| a.len() == 2).and_then(|a| {
                Some((
                    u32::try_from(a[0].as_u64()?).ok()?,
                    u32::try_from(a[1].as_u64()?).ok()?,
                ))
            });
            let Some((class, mask)) = pair else {
                bail!("line {}: gfx_ready pair {p}: use [class, mask]", n + 1);
            };
            r.masks.insert(class, mask);
        }
        out.insert(f, r);
    }
    Ok(out)
}

/// Reads and parses a `--gfx-ready` file ([`parse_gfx_ready`]).
pub fn read_gfx_ready(
    path: &Path,
    rate: d2_sim::tick::TickRate,
) -> Result<BTreeMap<u64, GfxReady>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("--gfx-ready {}", path.display()))?;
    parse_gfx_ready(&text, rate).with_context(|| format!("--gfx-ready {}", path.display()))
}

/// Reads and parses a send-clock file ([`parse_send_clock`]).
pub fn read_send_clock(path: &Path, rate: d2_sim::tick::TickRate) -> Result<BTreeMap<i32, u32>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("--send-clock {}", path.display()))?;
    parse_send_clock(&text, rate).with_context(|| format!("--send-clock {}", path.display()))
}

/// Parses a player-seed file: a
/// `# player-seed 1` first line, then `frame<TAB>lo<TAB>hi` rows (decimal).
pub fn parse_player_seed(
    text: &str,
    rate: d2_sim::tick::TickRate,
) -> Result<BTreeMap<i32, (u32, u32)>> {
    crate::recording::validate(
        crate::recording::text_rate(text).map_err(anyhow::Error::msg)?,
        rate,
    )
    .map_err(anyhow::Error::msg)?;
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("# player-seed 1") {
        bail!("player seed: the first line must be '# player-seed 1'");
    }
    let mut out = BTreeMap::new();
    for (n, line) in lines.enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t').map(str::trim);
        let row = (|| {
            let f = cols.next()?.parse().ok()?;
            let lo = cols.next()?.parse().ok()?;
            let hi = cols.next()?.parse().ok()?;
            cols.next().is_none().then_some((f, (lo, hi)))
        })();
        let Some((frame, seed)) = row else {
            bail!(
                "player seed line {}: '{line}': use frame<TAB>lo<TAB>hi",
                n + 2
            );
        };
        out.insert(frame, seed);
    }
    Ok(out)
}

/// Reads and parses a player-seed file ([`parse_player_seed`]).
pub fn read_player_seed(
    path: &Path,
    rate: d2_sim::tick::TickRate,
) -> Result<BTreeMap<i32, (u32, u32)>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("--player-seed {}", path.display()))?;
    parse_player_seed(&text, rate).with_context(|| format!("--player-seed {}", path.display()))
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Today's date (UTC) as `YYYY-MM-DD`, from the system clock.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    civil_date((secs / 86_400) as i64)
}

/// The proleptic Gregorian date of `days` since 1970-01-01.
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The game the dump runs: what `play` resolves from the same options.
pub struct DumpGame {
    /// Simulation rate selected for the run.
    pub tick_rate: d2_sim::tick::TickRate,
    pub data: GameData,
    pub character: Character,
    /// The game seed (`single_player::game_seed`).
    pub seed: u32,
    pub hardcore: bool,
    /// The 0x67 flags; `None`: the character's own.
    pub start_flags: Option<u32>,
    /// The `--poke` entries, in command-line order.
    pub pokes: Vec<Entry>,
    /// The `--input` steps.
    pub input: Option<Vec<input_script::Step>>,
    /// The `--send` entries, in command-line order.
    pub sends: Vec<SendEntry>,
    /// `--no-own-c2s` ids.
    pub no_own_c2s: Vec<u8>,
    pub early: Vec<Vec<u8>>,
    /// `--no-ping`.
    pub no_ping: bool,
    /// `--packets FILE` ([`super::packet_dump`]).
    pub packets: Option<PathBuf>,
    /// `--rng FILE` ([`super::rng_dump`]).
    pub rng: Option<PathBuf>,
    /// `--save-out FILE`: where the server's character writer puts the
    /// `.d2s` (`play`'s [`super::save::FileStore`]).
    pub save_out: Option<PathBuf>,
    /// `--client-out FILE` ([`super::client_state`]).
    pub client_out: Option<PathBuf>,
    /// `--client-grid` ([`DumpArgs::client_grid`]).
    pub client_grid: bool,
    /// `--send-clock FILE`, read ([`DumpArgs::send_clock`]); empty: none.
    pub send_clock: BTreeMap<i32, u32>,
    /// `--player-seed FILE`, read ([`DumpArgs::player_seed`]); empty: none.
    pub player_seed: BTreeMap<i32, (u32, u32)>,
    /// `--gfx-ready FILE`, read ([`DumpArgs::gfx_ready`]); empty: none
    /// (the dead draw's write-back runs as not ready).
    pub gfx_ready: BTreeMap<u64, GfxReady>,
}

impl DumpGame {
    /// Resolves `args` the way `d2-client play` resolves its command line
    /// (`main::play_once`): the character of `--save` (or the default),
    /// the difficulty, the seed, the hardcore bit of the save.
    pub fn resolve(args: &DumpArgs, data: GameData, game_dir: Option<&Path>) -> Result<Self> {
        // A save the load refuses plays the join as the game does: the 0x67
        // request, then S→C 0xB4.
        let refused = args
            .save
            .as_deref()
            .and_then(|p| single_player::refused_load(&data, p, args.difficulty));
        let mut start = play_start::resolve(
            &CliStart {
                save: if refused.is_some() {
                    None
                } else {
                    args.save.clone()
                },
                new: None,
                save_dir: None,
                difficulty: args.difficulty,
                hardcore: false,
            },
            &data,
            game_dir,
            None,
            None,
        )?;
        if let Some(c) = refused {
            start.character = c;
        }
        let seed = single_player::game_seed(&start.character, args.seed);
        // `play::run`: a loaded save's own status bit makes it hardcore.
        let hardcore = start.hardcore
            || super::save::base_save(&start.character).header.status
                & d2_formats::d2s::status::HARDCORE
                != 0;
        Ok(Self {
            tick_rate: args.tick_rate,
            data,
            character: start.character,
            seed,
            hardcore,
            start_flags: start.start_flags,
            pokes: args.pokes.clone(),
            input: args.input.clone(),
            sends: args.sends.clone(),
            no_own_c2s: args.no_own_c2s.clone(),
            early: args.early.clone(),
            no_ping: args.no_ping,
            packets: args.packets.clone(),
            rng: args.rng.clone(),
            save_out: args.save_out.clone(),
            client_out: args.client_out.clone(),
            client_grid: args.client_grid,
            send_clock: match &args.send_clock {
                Some(p) => read_send_clock(p, args.tick_rate)?,
                None => BTreeMap::new(),
            },
            player_seed: match &args.player_seed {
                Some(p) => read_player_seed(p, args.tick_rate)?,
                None => BTreeMap::new(),
            },
            gfx_ready: match &args.gfx_ready {
                Some(p) => read_gfx_ready(p, args.tick_rate)?,
                None => BTreeMap::new(),
            },
        })
    }
}

/// What a dump wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DumpReport {
    /// Server ticks run.
    pub ticks: u32,
    /// Snapshots written.
    pub snaps: u64,
}

/// The header fields of the run (all but `fields` / `gaps`, which come
/// from the game).
pub struct RunInfo {
    pub tool: String,
    pub date: String,
    pub command: String,
    pub save: Option<String>,
}

/// What the dump knows it does not match, beyond the unit fields: the
/// client side is the bridge alone.
pub const RUN_GAPS: [&str; 1] = [
    "client: headless bridge (no UI or visibility art); the only C->S messages are 0x67, \
     the model's own answers (0x6B, 0x5F, 0x28's 0x2F / 0x30 and its dialog branch's 0x31 from the headless UI), the --send messages and the --input clicks (world-click dispatcher \
     with the play preview's hover pick, the local player at the play preview's walk \
     prediction, held repeat once per server frame; keys: belt 1-4, run lock, weapon swap, \
     speech only), so a run \
     where a reference client sends anything else differs from the first such tick",
];

/// The server ticks a refused load runs.
const REFUSED_TICKS: u32 = 2;

/// Runs `game` for `ticks` server ticks and writes the `state-1` lines to
/// `out` (one snapshot per tick whose frame is a multiple of `every`).
pub fn dump<W: Write>(
    game: DumpGame,
    ticks: u32,
    every: u32,
    info: &RunInfo,
    out: &mut W,
) -> Result<DumpReport> {
    let every = every.max(1);
    let rate = game.tick_rate;
    let mut clock_step = 0;
    // A refused load ends the game at the second tick: 0xB4 goes out in
    // the first tick's input phase and the client is removed.
    let ticks = if matches!(game.character, single_player::Character::Refused(_)) {
        ticks.min(REFUSED_TICKS)
    } else {
        ticks
    };
    let ms = Arc::new(AtomicU32::new(START_MS));
    let client_data = ClientData::of(&game.data)?;
    let speeds = single_player::walk_speeds(&game.data, &game.character)?;
    // The string tables the NPC dialogs read their text from: without them
    // every dialog opens empty and its scroll never ends.
    // `expansion_installed` (`d2exp.mpq` is found in the install), read
    // from the archives as `play` reads it (`app/ui.rs` `UiParts::live`). The
    // `inventory.bin` `inv` rectangles, as `play` passes them (`app/ui.rs`
    // `UiParts::live`): without them an open right panel's area is empty, so a
    // click on it reaches the world handlers instead of the panel
    // (seen with a click on the right panel at Warriv).
    let (strings, expansion_installed, inv_areas) = {
        let GameData::Live(live) = &game.data;
        let strings =
            super::strings::TableStrings::load(live.archives.as_ref(), super::strings::LANG)
                .map_err(anyhow::Error::msg)?;
        (
            strings,
            d2_data::bin::TableFiles::lod(live.archives.as_ref()),
            super::ui::live_inv_areas(live.archives.as_ref()).map_err(anyhow::Error::msg)?,
        )
    };
    // The waypoint menu's rows read the levels' waypoint indexes and names, as
    // `play` installs them: without them the menu has no rows and a row click
    // sends no C→S 0x49.
    let waypoint_map = single_player::client_waypoint_map(&game.data);
    let level_names = single_player::client_level_names(&game.data);
    // The HUD's skill tables: without them the select panel has no
    // icons, so its click and a hotkey assign find no record.
    let hud_tables = {
        let GameData::Live(live) = &game.data;
        super::hud::hud_tables(live.archives.as_ref()).map_err(anyhow::Error::msg)?
    };
    // The unit art of the cel offsets (installed on the bridge below).
    let cel_art = cel_offset_art(&game.data)
        .map_err(|e| eprintln!("cel offsets: unit art not loaded, every request fails: {e}"))
        .ok();
    // Only a script clicks: without one the hover is never read.
    let hover_art = match game.input.as_ref().filter(|s| !s.is_empty()) {
        Some(_) => headless_art(&game.data)
            .map_err(|e| eprintln!("input: hover art not loaded, feet-box pick: {e}"))
            .ok(),
        None => None,
    };
    // `skill` / `cast` steps name skills by the player's `skills.txt`.
    let skill_names = match game.input.as_ref().filter(|s| input_script::uses_skills(s)) {
        Some(_) => {
            let GameData::Live(live) = &game.data;
            crate::world_view::script_skills::SkillNames::load(live.archives.as_ref())
                .map_err(|e| eprintln!("input: skill names not loaded, ids only: {e}"))
                .ok()
                .flatten()
        }
        None => None,
    };
    // `play`'s character writer (`play::run`), for a `--send "<f> hex 69"`.
    let store = match &game.save_out {
        Some(path) => {
            let GameData::Live(live) = &game.data;
            let mut base = super::save::base_save(&game.character);
            if game.hardcore {
                base.header.status |= d2_formats::d2s::status::HARDCORE;
            }
            Some(super::save::FileStore {
                path: path.clone(),
                base,
                tables: Arc::new(live.save.clone()),
                appearance: Some(Arc::new(
                    super::save::appearance_tables(&live.tables.fixed)
                        .map_err(anyhow::Error::msg)?,
                )),
                codec: Arc::new(super::single_player::D2sCodec),
            })
        }
        None => None,
    };
    let mut rng = match &game.rng {
        Some(p) => Some(super::rng_dump::RngDump::create(p, info, game.seed, rate)?),
        None => None,
    };
    let (mut link, _started) = super::rng_dump::start_with(
        game.data,
        game.seed,
        game.character.clone(),
        StepClock(ms.clone()),
        rng.is_some(),
    )?;
    link.with(move |l| {
        let host = l.host_mut();
        host.game.game.tick_rate = rate;
        host.driver = d2_server::host::TickDriver::new(rate);
    })?;
    if game.hardcore {
        link.with(|l| l.host_mut().game.events.action.hooks().x.hardcore = true)?;
    }
    if !game.send_clock.is_empty() {
        let clock = game.send_clock.clone();
        link.with(move |l| l.host_mut().set_send_clock(clock))?;
    }
    let saving = store.is_some();
    if let Some(store) = store {
        link.with(move |l| l.host_mut().game.set_storage(Box::new(store)))?;
    }
    let mut packets = match &game.packets {
        Some(p) => Some(super::packet_dump::PacketDump::create(
            &mut link, p, info, game.seed,
        )?),
        None => None,
    };
    let mut client_out = match &game.client_out {
        Some(p) => {
            let mut f = std::io::BufWriter::new(std::fs::File::create(p)?);
            writeln!(
                f,
                "{}",
                super::client_state::header(
                    &info.tool,
                    &info.date,
                    &info.command,
                    game.client_grid,
                    rate
                )
            )?;
            Some(f)
        }
        None => None,
    };
    let mut client_pre = String::new();
    let (mut fields, mut gaps) = link.with(|l| state::coverage_world(&l.host().game.events))?;
    // `q`: the players' quest records, kept by the app's rest (StateSource below)
    fields.extend(state::HOST_FIELDS.iter().map(|k| (*k).to_owned()));
    gaps.extend(RUN_GAPS.iter().map(|g| (*g).to_owned()));
    let header = state::Header {
        side: "d2rust".into(),
        tool: info.tool.clone(),
        date: info.date.clone(),
        command: info.command.clone(),
        fields,
        gaps,
        save: info.save.clone(),
        seed: Some(game.seed),
        tick_rate: rate,
    };
    writeln!(out, "{}", header.to_json_line())?;

    // The pokes due after a tick run at the tick end
    // (the poke hook's point), after that frame's snapshot; a `goto` steps
    // there on every later frame until it ends.
    let (tick_end_entries, rest) = pokes::split_tick_end(game.pokes);
    let tick_end = Arc::new(Mutex::new(TickEndOut::default()));
    if !tick_end_entries.is_empty() {
        tick_end.lock().unwrap_or_else(|e| e.into_inner()).pending = tick_end_entries.len();
        let shared = tick_end.clone();
        let mut entries = tick_end_entries;
        link.with(move |l| {
            l.set_tick_end(Box::new(move |h| {
                let frame = h.game.game.frame;
                let (due, later): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut entries)
                    .into_iter()
                    .partition(|e| pokes::due_after(e.when, None).is_some_and(|f| frame >= f));
                entries = later;
                let mut out = shared.lock().unwrap_or_else(|e| e.into_inner());
                if due.is_empty() && out.walking.is_empty() {
                    return;
                }
                out.snapshot = Some((frame, snapshot_host(h)));
                // `goto` walks step first, then the pokes due now.
                let due = std::mem::take(&mut out.walking)
                    .into_iter()
                    .map(|(e, w)| (e, Some(w)))
                    .chain(due.into_iter().map(|e| (e, None)));
                for (i, (e, walk)) in due.enumerate() {
                    let When::Frame(f) = e.when else { continue };
                    let line = match (&e.op, walk) {
                        (PokeOp::Directive(Directive::Goto(t)), w) => {
                            let (r, w) = pokes::goto_now(&mut h.game, *t, w.unwrap_or_default());
                            if r == PokeResult::Pending {
                                out.walking.push((e, w));
                                continue;
                            }
                            // a walk's record carries the frame of its last step
                            pokes::record_line_steps(frame + 1, i, &e.op, &r, Some(w.steps))
                        }
                        _ => {
                            let r = pokes::apply_on_host(h, &e.op);
                            pokes::record_line(f, i, &e.op, &r)
                        }
                    };
                    out.lines.push(line);
                }
                pokes::queue_sent_now(h);
                out.after = Some(snapshot_host(h));
                out.pending = entries.len() + out.walking.len();
            }))
        })?;
    }
    let mut link = link;
    link.set_step_paths(|l| server_paths_of(&snapshot_host(l.host())));
    let link = PredictLink::new(link);
    let tap = link.tap();
    let mut bridge = Bridge::new(link)?;
    client_data.install(&mut bridge);
    bridge.set_ping(!game.no_ping);
    // The view's unit art the client model reads cel offsets from: `play` draws
    // every unit, so its COF and layer files are resident; the dump makes them
    // resident after each pass's draw, as the view's loader does.
    let mut cel_art = cel_art.map(|(art, f)| {
        bridge.set_cel_offset(Some(f));
        art
    });
    if !game.gfx_ready.is_empty() {
        bridge.set_gfx_ready(Some(Arc::new(game.gfx_ready.clone())));
    }
    let mut request = single_player::create_request_for(&game.character);
    if let Some(f) = game.start_flags {
        request.flags = f;
    }
    bridge.send(&request)?;
    for m in &game.early {
        bridge.send_bytes(m)?;
    }
    bridge.set_drop_own(game.no_own_c2s.clone());
    let mut ui = DialogUi::new(strings, expansion_installed, inv_areas)?;
    ui.ui.set_waypoint_map(waypoint_map);
    ui.ui.set_level_names(level_names);
    ui.ui.set_hud_tables(hud_tables);
    let (mut ran, mut snaps, mut idle) = (0u32, 0u64, 0u32);
    let mut first = true;
    // The frame the last tick ran (0: none yet) and the pokes still to run.
    let mut last_frame = 0i32;
    // The frame whose recorded player seed was set (`--player-seed`).
    let mut seeded_for = 0i32;
    let mut pending = rest;
    // The client part follows the server's walk and point as the play
    // preview does, with or without an input script.
    let mut input = Some(
        Headless::new(game.input.unwrap_or_default())
            .map_err(anyhow::Error::msg)?
            .with_prediction(tap, speeds),
    )
    .map(|h| match hover_art {
        Some(art) => h.with_art(art),
        None => h,
    })
    .map(|h| match skill_names {
        Some(names) => h.with_skill_names(names),
        None => h,
    });
    let mut input_notes = Vec::new();
    let mut to_send = game.sends;
    let mut send_notes = Vec::new();
    let (mut joined, mut notes_left) = (false, None::<u32>);
    // The tick count when the single-player pause ended the run.
    let mut paused_at = None::<u32>;
    // A fatal exit of the client model: the run ends after this tick's
    // snapshot, as a reference dump does.
    let mut fatal = None::<String>;
    while ran < ticks {
        run_due_pokes(&mut bridge, &mut pending, last_frame, out)?;
        if let Some(h) = input.as_mut() {
            h.set_open_mode(ui.ui.open_mode().get());
            for l in h.apply_with(&mut bridge, last_frame, &mut |b, e| ui.route(b, e))? {
                eprintln!("input: {l}");
                input_notes.push(format!("input: {l}"));
            }
            // The click's mode request sets the client player's walk in the
            // same pass: the
            // prediction takes the walks sent so far now, not after the
            // server frame (the snapshot below is of this pass).
            h.observe(bridge.world(), false);
            h.sync_local(&mut bridge);
        }
        run_due_sends(&mut bridge, &mut to_send, last_frame, out, &mut send_notes)?;
        if !std::mem::replace(&mut first, false) {
            clock_step += 1;
            ms.store(step_ms(rate, clock_step), Ordering::SeqCst);
        }
        bridge.set_now(ms.load(Ordering::SeqCst));
        // The single-player pause: with ui 9 (the Esc menu) or 11 open the pass
        // runs no server tick, as `play`'s `pause_frame`. With no input step
        // left nothing closes it: the run ends here, as a reference dump
        // does (no frame after the pause).
        let paused = ui.ui.is_open(9) || ui.ui.is_open(11);
        bridge.set_paused(paused);
        if paused && input.as_ref().is_none_or(|h| h.pending() == 0) {
            paused_at = Some(ran);
            break;
        }
        // The recorded seed of the snapshot this pass writes, set once per
        // frame before it: the frame's draws
        // on the player's own seed step from the reference value.
        let f = last_frame + 1;
        if seeded_for != f {
            seeded_for = f;
            if let Some(&(lo, hi)) = game.player_seed.get(&f) {
                bridge.set_local_seed(lo, hi);
            }
        }
        let t0 = super::perf::enabled().then(std::time::Instant::now);
        if client_out.is_some() {
            client_pre = if game.client_grid {
                let fp = super::client_state::Footprints::of(bridge.world(), bridge.inputs());
                super::client_state::units_json_with(bridge.world(), fp.as_ref())
            } else {
                super::client_state::units_json(bridge.world())
            };
        }
        let rejected_before = bridge.log().rejected.len();
        let report = bridge.frame()?;
        if let Some(t0) = t0 {
            super::perf::record_bridge_frame(t0, report.ticked);
        }
        let outputs = bridge.take_outputs();
        ui.deliver(&mut bridge, &outputs)?;
        // The pass's draw: its camera is the
        // unit origin the next client update's missile function 2 reads.
        if let Some(cam) = headless_draw_camera(bridge.world(), ui.ui.open_mode()) {
            bridge.set_unit_origin(&cam);
        }
        if let Some((loader, assets)) = cel_art.as_mut() {
            loader.ensure(bridge.world(), assets);
        }
        if fatal.is_none() {
            fatal = bridge.log().rejected[rejected_before..]
                .iter()
                .find_map(|r| r.error.fatal_exit());
        }
        for m in bridge.take_dropped() {
            let hex: Vec<String> = m.iter().map(|b| format!("{b:02x}")).collect();
            let line = format!(
                "own c2s dropped: frame {}: {}",
                last_frame + 1,
                hex.join(" ")
            );
            eprintln!("{line}");
            send_notes.push(line);
        }
        // Save and Exit (C→S 0x69) took the client out of the game: the
        // server wrote the file in its leave.
        if saving {
            let in_game = bridge.world().in_game;
            if in_game {
                joined = true;
            } else if joined {
                notes_left = Some(ran);
                break;
            }
        }
        if let Some(h) = input.as_mut() {
            for l in h.after_frame(&mut bridge, report.ticked)? {
                eprintln!("input: {l}");
                input_notes.push(format!("input: {l}"));
            }
            h.sync_local(&mut bridge);
        }
        if let Some(p) = packets.as_mut() {
            p.drain()?;
        }
        if let Some(r) = rng.as_mut() {
            r.drain(&mut bridge)?;
        }
        if !report.ticked {
            idle += 1;
            if idle > MAX_IDLE_STEPS {
                bail!("no server tick in {MAX_IDLE_STEPS} steps (after {ran} ticks)");
            }
            continue;
        }
        idle = 0;
        ran += 1;
        // The tick-end pokes' records, and the snapshot taken before them;
        // the records are written after that snapshot's line (between the
        // two snapshots, as `record_state.py` writes them).
        let copy_from;
        let poke_lines: Vec<String>;
        let held = {
            let mut t = tick_end.lock().unwrap_or_else(|e| e.into_inner());
            poke_lines = std::mem::take(&mut t.lines);
            copy_from = t.after.take();
            t.snapshot.take()
        };
        let s = match held {
            Some((_, s)) => s,
            None => bridge.state_snapshot()?,
        };
        last_frame = s.frame;
        // The single-player server copy at the end of the
        // client update: the server monsters'
        // path state after this tick.
        let mut copies = server_paths_of(copy_from.as_ref().unwrap_or(&s));
        // Each server player's path new direction (+0x65) after this tick
        // (`ServerPath::new_dir`), for the client step's aim.
        let new_dirs = bridge.player_new_directions()?;
        for p in copies.iter_mut().filter(|p| p.key.unit_type == 0) {
            p.new_dir = new_dirs
                .iter()
                .find(|(g, _)| *g == p.key.guid)
                .map(|(_, d)| *d);
            // The local player aims at its own client path (the path
            // on the prediction's path), not the server's.
            if bridge.world().local_player == Some(p.key) {
                if let Some(d) = input.as_ref().and_then(|h| h.predicted_new_direction()) {
                    p.new_dir = Some(d);
                }
            }
        }
        bridge.copy_server_paths(&copies);
        if s.frame.rem_euclid(every as i32) == 0 {
            writeln!(out, "{}", s.to_json_line())?;
            snaps += 1;
            if let Some(f) = client_out.as_mut() {
                writeln!(
                    f,
                    "{}",
                    super::client_state::snap_line(&client_pre, s.frame)
                )?;
            }
        }
        for line in poke_lines {
            eprintln!("poke: at the tick end: {line}");
            writeln!(out, "{line}")?;
        }
        if fatal.is_some() {
            break;
        }
    }
    if let Some(f) = client_out.as_mut() {
        f.flush()?;
    }
    let mut notes = vec![if rate.is_base() {
        format!("{ran} server ticks, clock {STEP_MS} ms per step from {START_MS} ms, every {every}")
    } else {
        format!(
            "{ran} server ticks, clock {} Hz from {START_MS} ms, every {every}",
            rate.hz()
        )
    }];
    if let Some(n) = notes_left {
        notes.push(format!("left the game after {n} ticks (Save and Exit)"));
    }
    if let Some(n) = paused_at {
        eprintln!("state-dump: paused (ui 9 or 11 open) after {n} ticks: the run ends");
        notes.push(format!(
            "paused after {n} ticks (ui 9 or 11 open, single-player pause)"
        ));
    }
    if let Some(what) = fatal {
        notes.push(format!(
            "fatal exit after {ran} ticks: {what} (exit code 0xffffffff)"
        ));
    }
    notes.extend(input_notes);
    notes.extend(send_notes);
    notes.extend(ui.notes);
    for e in &to_send {
        eprintln!(
            "send: not reached in {ran} ticks: frame {} {}",
            e.frame,
            e.text()
        );
        notes.push(format!("send not reached: frame {} {}", e.frame, e.text()));
    }
    if let Some(n) = input.as_ref().map(Headless::pending).filter(|&n| n > 0) {
        eprintln!("input: {n} step(s) not reached in {ran} ticks");
        notes.push(format!("input: {n} step(s) not reached"));
    }
    let (unreached, tick_end_walks) = tick_end.lock().map_or((0, Vec::new()), |mut t| {
        let walks = std::mem::take(&mut t.walking);
        (t.pending.saturating_sub(walks.len()), walks)
    });
    let walking = tick_end_walks;
    if unreached > 0 {
        eprintln!("poke: {unreached} tick-end poke(s) not reached in {ran} ticks");
        notes.push(format!("poke not reached: {unreached} tick-end poke(s)"));
    }
    for e in &pending {
        eprintln!("poke: not reached in {ran} ticks: {:?} {}", e.when, e.op);
        notes.push(format!("poke not reached: {:?} {}", e.when, e.op));
    }
    // A `goto` still walking at the end is `failed`.
    for (i, (e, w)) in walking.iter().enumerate() {
        let f = last_frame + 1;
        let r = d2_sim::poke::PokeResult::Failed;
        let line = pokes::record_line_steps(f, i, &e.op, &r, Some(w.steps));
        eprintln!("poke: goto not finished in {ran} ticks: {line}");
        notes.push(format!("goto not finished: {}", e.op));
        writeln!(out, "{line}")?;
    }
    if let Some(p) = packets {
        p.finish(&notes)?;
    }
    if let Some(r) = rng {
        r.finish(&notes)?;
    }
    writeln!(out, "{}", state::footer_line(snaps, &notes))?;
    out.flush()?;
    Ok(DumpReport { ticks: ran, snaps })
}

/// Runs every pending poke due after `last_frame` (absolute `f` with f − 1 ≤
/// `last_frame`) through the bridge, in order, writing a `poke` line each and
/// printing it to stderr. Only the pokes due before the first tick get here;
/// the others run at the tick end ([`pokes::split_tick_end`]). The camera of a
/// headless pass's draw: the local player's model position, as
/// [`input_script::script_camera`], in the UI's open mode, no shake; `None`
/// without a local player.
fn headless_draw_camera(
    world: &crate::bridge::world::ClientWorld,
    mode: crate::rules::camera::OpenMode,
) -> Option<crate::rules::camera::Camera> {
    use crate::rules::camera::{moving_to_client, Camera, FrameSize};
    world.local()?;
    let (x, y) = world.local_position()?;
    Some(Camera::new(
        FrameSize::play(),
        mode,
        moving_to_client(x, y),
        (0, 0),
    ))
}

fn run_due_pokes<W: Write>(
    bridge: &mut Bridge<DumpLink>,
    pending: &mut Vec<Entry>,
    last_frame: i32,
    out: &mut W,
) -> Result<()> {
    let (due, later): (Vec<Entry>, Vec<Entry>) = std::mem::take(pending)
        .into_iter()
        .partition(|e| pokes::due_after(e.when, None).is_some_and(|f| last_frame >= f));
    *pending = later;
    for (i, e) in due.iter().enumerate() {
        // state-dump takes absolute --poke frames only (never a Tick entry)
        let When::Frame(f) = e.when else { continue };
        let r = bridge.poke(&e.op)?;
        let line = pokes::record_line(f, i, &e.op, &r);
        eprintln!("poke: before frame {f}: {}: {line}", e.op);
        writeln!(out, "{line}")?;
    }
    Ok(())
}

/// Injects every pending `--send` due after `last_frame` (f − 1 ≤
/// `last_frame`) through the bridge, in order, writing a `send` line each,
/// printing it to stderr and keeping it for the footer notes.
fn run_due_sends<W: Write>(
    bridge: &mut Bridge<DumpLink>,
    pending: &mut Vec<SendEntry>,
    last_frame: i32,
    out: &mut W,
    notes: &mut Vec<String>,
) -> Result<()> {
    let (due, later): (Vec<SendEntry>, Vec<SendEntry>) = std::mem::take(pending)
        .into_iter()
        .partition(|e| last_frame >= e.frame - 1);
    *pending = later;
    for (i, e) in due.iter().enumerate() {
        let r = bridge.inject(&e.msg)?;
        let line = sends::record_line(e.frame, i, &e.text(), &r);
        eprintln!("send: before frame {}: {line}", e.frame);
        notes.push(format!("send: {line}"));
        writeln!(out, "{line}")?;
    }
    Ok(())
}

/// The server thread's snapshot, taken on the server thread between two
/// frames, with each player's `q`: the quest
/// record of the game's difficulty from the rest's per-player quests
/// (`AppRest::quests`). Reads only.
impl<C: Clock + Send + 'static> StateSource for ThreadLink<Link<C>> {
    type Error = super::server_thread::ThreadStopped;
    fn state_snapshot(&mut self) -> Result<state::StateSnapshot, Self::Error> {
        self.with(|l| snapshot_host(l.host()))
    }

    fn player_new_directions(&mut self) -> Result<Vec<(u32, u8)>, Self::Error> {
        self.with(|l| {
            let h = l.host();
            state::player_new_directions_world(&h.game.game, &h.game.events)
        })
    }
}

/// What the tick-end hook hands back to the dump loop: the poke records
/// written at the tick end, the snapshot of that frame taken before them
/// (`record_state.py` snapshots at the same hook before its pokes),
/// the `goto` walks still stepping there and how many tick-end pokes
/// (walks included) are still to run.
#[derive(Default)]
struct TickEndOut {
    lines: Vec<String>,
    /// The `goto` walks still stepping at the tick end.
    walking: Vec<(Entry, GotoWalk)>,
    snapshot: Option<(i32, state::StateSnapshot)>,
    /// The server after this tick's pokes (the single-player copy reads
    /// the live server).
    after: Option<state::StateSnapshot>,
    pending: usize,
}

/// The server monsters' path state of `s` for the single-player server
/// copy.
pub(super) fn server_paths_of(
    s: &state::StateSnapshot,
) -> Vec<crate::bridge::critter_path::ServerPath> {
    s.units
        .iter()
        .filter(|u| u.ut <= 1)
        .filter_map(|u| {
            Some(crate::bridge::critter_path::ServerPath {
                key: crate::bridge::world::UnitKey::new(u.ut, u.g),
                mode: u.m?,
                x: u.x? as u16,
                y: u.y? as u16,
                xf: u.xf?,
                yf: u.yf?,
                tx: u.tx?,
                ty: u.ty?,
                copy: u.path_copy.clone(),
                new_dir: None,
            })
        })
        .collect()
}

/// The snapshot of the host's game (the [`StateSource`] above, and the
/// tick-end hook's).
pub(super) fn snapshot_host<C: Clock>(h: &pokes::ServerHost<C>) -> state::StateSnapshot {
    let sim = &h.game;
    let mut s = state::snapshot_world(&sim.game, &sim.events);
    if let Some(inv) = sim.world.inventory.as_ref() {
        overlay_item_places(&mut s, &inv.state);
        overlay_item_owners(
            &mut s,
            &inv.state,
            &sim.game.lists,
            &sim.events.action.sys.units,
        );
    }
    let d = usize::from(state::difficulty_world(&sim.events)).min(2);
    for (id, q) in &sim.world.rest.quests {
        if let Some(e) = sim.game.lists.unit(*id) {
            s.set_quests(e.guid, state::quest_words(&q.flags[d]));
        }
    }
    // A player corpse (mode 17) holds an all-zero quest record: the death
    // corpse's spawn runs the
    // player-data init, which allocates three zeroed records. D2RUST keeps no
    // record for it, so the export writes the empty one.
    for u in &mut s.units {
        if u.ut == d2_sim::units::lists::UnitType::Player as u8 && u.m == Some(17) && u.q.is_none()
        {
            u.q = Some(Vec::new());
        }
    }
    s
}

/// An item in an inventory has a static path whose x, y are its place
/// (the cell of a page, the belt slot, the body location) and whose direction
/// is 0 (the state snapshot format).
/// D2RUST keeps the place in the inventory model, not in a path record, so the
/// export reads it there, also for an item that just left the ground (its
/// ground path is stale).
fn overlay_item_places(snap: &mut state::StateSnapshot, inv: &d2_sim::wiring::inventory::InvState) {
    use d2_sim::items::moves::mode;
    for u in snap.units.iter_mut().filter(|u| u.ut == 4) {
        // A monster's equipment (owner none, equipped) has its body
        // location in its own static path, which the snapshot already
        // carries; a store's stock (owner none, stored) is placed in
        // the store's page like a player's item.
        let no_path_place = u.x.is_none();
        let placed = inv.items.values().find(|d| {
            d.guid == u.g
                && match d.mode {
                    mode::STORED => true,
                    // An item taken off a hireling stays equipped, unlinked,
                    // at its body location (the snapshot carries no place
                    // for it: `hire-items-kashya` frame 50, x 1).
                    mode::EQUIPPED | mode::BELT => {
                        d.owner_guid != d2_sim::items::inventory::NO_GUID
                            || d.inv.is_some()
                            || (no_path_place && d.mode == mode::EQUIPPED)
                    }
                    // On the cursor the static path reads (0, 0)
                    // (an item taken from a hireling's gear). A worn
                    // item taken off by the load's runeword clean-up is
                    // unlinked on the cursor mode and keeps its place (x = the
                    // body location, as a runeword item worn at load: x 1, y 0).
                    mode::CURSOR => {
                        d.inv.is_some()
                            || (no_path_place && d.owner_guid != d2_sim::items::inventory::NO_GUID)
                    }
                    // A socketed filler: its path x is the socket index, y 0
                    // (socket fillers).
                    mode::SOCKETED => d.inv.is_some(),
                    _ => false,
                }
        });
        if let Some(d) = placed {
            u.x = u32::try_from(d.x).ok();
            u.y = u32::try_from(d.y).ok();
            u.d = Some(0);
            // The static path's room is 0: no `lv`.
            u.lv = None;
        }
    }
}

/// `own` of an item: the holder of the inventory
/// the item is linked into (item data +0x5C -> inventory +0x08 owner
/// unit); absent on the ground (no inventory) or with a 0 GUID. D2RUST
/// keeps the link in the inventory model.
///
/// A store item (vendor flag, unit +0xC8 bit 2)
/// reads absent: the store items of Akara
/// have no `own` while the bought copy in the player's
/// inventory has the player's GUID. The store's inventory has no owner unit
/// ("new NPC inventory").
fn overlay_item_owners(
    snap: &mut state::StateSnapshot,
    inv: &d2_sim::wiring::inventory::InvState,
    lists: &d2_sim::units::UnitLists,
    units: &d2_sim::units::record::Units,
) {
    for u in snap.units.iter_mut().filter(|u| u.ut == 4) {
        let found = inv.items.iter().find(|(_, d)| d.guid == u.g);
        let vendor = found.is_some_and(|(id, _)| {
            units
                .get(*id)
                .is_some_and(|r| r.flags2 & d2_sim::items::moves::deferred::VENDOR_ITEM != 0)
        });
        let holder = found
            .and_then(|(_, d)| d.inv)
            .and_then(|o| lists.unit(o))
            .map(|e| e.guid);
        // A monster's equipment has no inventory model: the snapshot's
        // own holder (sim `owner`) stays.
        if vendor {
            u.own = None;
        } else if holder.is_some() {
            u.own = holder.filter(|&g| g != 0);
        }
    }
}

/// The bridge-level client data `play` installs (`play::add_live_client`):
/// what the model's own answers can depend on.
struct ClientData {
    drlg: crate::bridge::drlg::DrlgSource,
    levels: Vec<crate::bridge::world::LevelRow>,
    objects: Vec<crate::bridge::objects::ObjClientRow>,
    skills: Vec<crate::bridge::world::SkillRow>,
    class_skills: Vec<[u16; 10]>,
    class_velocities: Vec<[u8; 2]>,
    class_vitals: Vec<[u8; 3]>,
    stat_ops: Vec<Vec<(u16, u8)>>,
    skill_tables: d2_sim::skills::SkillTables,
    units: crate::bridge::world::UnitRows,
    player_anims: super::anim_names::ClientPlayerAnims,
    item_tables: Arc<d2_sim::items::ItemTables>,
}

impl ClientData {
    fn of(data: &GameData) -> Result<Self> {
        let GameData::Live(d) = data;
        let archives = d.archives.as_ref();
        Ok(Self {
            drlg: single_player::client_drlg_source(data),
            levels: single_player::client_level_rows(data),
            objects: single_player::client_object_rows(data),
            skills: single_player::client_skill_rows(archives)?,
            class_skills: single_player::client_class_skills(archives)?,
            class_velocities: single_player::client_class_velocities(archives)?,
            class_vitals: single_player::client_class_vitals(archives)?,
            stat_ops: single_player::client_stat_ops(archives)?,
            skill_tables: single_player::client_skill_tables(archives)?,
            units: {
                let mut u = single_player::client_unit_rows(archives)?;
                single_player::client_monster_anims(archives, &mut u)?;
                u
            },
            player_anims: single_player::client_player_anims(data)?,
            // The item type test of the use state (`bridge::use_state`).
            item_tables: Arc::new(d.tables.item_tables().map_err(|e| anyhow::anyhow!("{e}"))?),
        })
    }

    fn install<L: crate::bridge::link::ServerLink>(self, b: &mut Bridge<L>) {
        b.set_drlg_source(Some(self.drlg));
        b.set_levels(self.levels);
        b.set_object_rows(self.objects);
        b.set_skill_rows(self.skills);
        b.set_class_skills(self.class_skills);
        b.set_class_velocities(self.class_velocities);
        b.set_class_vitals(self.class_vitals);
        b.set_stat_ops(self.stat_ops);
        b.set_skill_tables(Arc::new(self.skill_tables));
        b.set_unit_rows(self.units);
        b.set_player_anims(Arc::new(self.player_anims));
        b.set_item_tables(Arc::new(super::items::TableDecoder(self.item_tables)));
        b.set_high_light_quality(true);
    }
}

/// The unit art loader the dump keeps resident after each pass, and the
/// cel offset reader over it, from the
/// user's archives as `play` reads them.
fn cel_offset_art(
    data: &GameData,
) -> Result<(
    (
        crate::world_view::unit_assets::UnitArtLoader,
        crate::world_view::ViewAssets,
    ),
    crate::bridge::world::CelOffsetFn,
)> {
    use crate::world_view::unit_assets::{cel_offset_fn, SharedUnitArt, UnitArtLoader, UnitLooks};
    let GameData::Live(d) = data;
    let looks = Arc::new(
        UnitLooks::live(d.archives.as_ref()).map_err(|e| anyhow::anyhow!("unit tables: {e}"))?,
    );
    let art = SharedUnitArt::default();
    let f = crate::bridge::world::CelOffsetFn::new(cel_offset_fn(looks.clone(), art.clone()));
    let loader = UnitArtLoader {
        source: d.archives.source(),
        looks,
        art,
    };
    let assets = crate::world_view::ViewAssets::new(super::play::unspecified_palette());
    Ok(((loader, assets), f))
}

/// The art of the headless hover pick: the unit
/// tables and item art rows of the user's archives, read as `play` reads
/// them.
fn headless_art(data: &GameData) -> Result<input_script::HeadlessArt> {
    let GameData::Live(d) = data;
    let looks = crate::world_view::unit_assets::UnitLooks::live(d.archives.as_ref())
        .map_err(|e| anyhow::anyhow!("unit tables: {e}"))?;
    let items = super::items::item_parts(d.archives.as_ref()).map_err(anyhow::Error::msg)?;
    Ok(input_script::HeadlessArt::new(
        d.archives.source(),
        looks,
        items.art,
        crate::world_view::ViewAssets::new(super::play::unspecified_palette()),
    ))
}

/// `d2-client state-dump...`: loads the user's files (`--game-dir` or
/// `$D2_GAME_DIR`), resolves the game as `play`, runs the dump into `--out`.
/// `command`: the argv, joined.
pub fn run(args: &DumpArgs, command: &str) -> Result<DumpReport> {
    let dir = args
        .game_dir
        .clone()
        .or_else(|| std::env::var_os("D2_GAME_DIR").map(PathBuf::from));
    let data = GameData::select(dir.as_deref())?;
    let game = DumpGame::resolve(args, data, dir.as_deref())?;
    let info = RunInfo {
        tool: format!("d2-client state-dump {}", env!("CARGO_PKG_VERSION")),
        date: args.date.clone().unwrap_or_else(today),
        command: command.to_owned(),
        save: args.save.as_ref().map(|p| p.display().to_string()),
    };
    if let Some(parent) = args.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let file = std::fs::File::create(&args.out)
        .with_context(|| format!("creating {}", args.out.display()))?;
    let mut w = std::io::BufWriter::new(file);
    dump(game, args.ticks, args.every, &info, &mut w)
}

/// The UI layer's part of the client the dump runs: the panel UI
/// (`ui/original.rs`, headless: no panels drawn, nothing shown) takes the
/// bridge's outputs in list order, as `play`'s output dispatcher does
/// (`world_view::present::deliver`), so the answer of 0x28's dialog branch
/// (case B2: C→S 0x31 right after the model's 0x2F) is sent as the game's client
/// sends it. Audio and effects outputs have no consumer here.
struct DialogUi {
    ui: crate::ui::original::OriginalUi,
    /// The root the panels are installed in: `--input` events go through
    /// it first, as `play`'s `run_ui_with` does (a panel key or a click on
    /// the control panel is taken there, not by the world).
    root: crate::ui::root::UiRoot,
    /// The UI errors met, once each (footer notes).
    notes: Vec<String>,
    /// The string tables the dialog texts come from.
    strings: super::strings::TableStrings,
}

impl DialogUi {
    fn new(
        strings: super::strings::TableStrings,
        expansion_installed: bool,
        inv_areas: Vec<crate::ui::original::InvArea>,
    ) -> Result<Self> {
        // `expansion_installed` (`d2exp.mpq` is found in the install), read
        // from the archives by the caller as `play` does.
        let config = crate::ui::original::UiConfig {
            screen: crate::ui::layout::Screen::play(),
            expansion_installed,
        };
        let mut ui = crate::ui::original::OriginalUi::new(config, Some(inv_areas))
            .map_err(|e| anyhow::anyhow!("game UI: {e:?}"))?;
        // Reference dumps run `-ns` (no sound device): no speech handle is
        // ever set, so the dialog panel never waits for speech and the scroll
        // runs from the open.
        ui.set_speech_stalls(false);
        let mut root = crate::ui::root::UiRoot::new(Box::new(crate::ui::NoPanelRules));
        ui.install(&mut root)
            .map_err(|e| anyhow::anyhow!("game UI install: {e:?}"))?;
        Ok(Self {
            ui,
            root,
            notes: Vec::new(),
            strings,
        })
    }

    /// The `--input` events of a pass through the panels; the events no
    /// panel took (for the world handlers).
    fn route(
        &mut self,
        bridge: &mut Bridge<DumpLink>,
        events: Vec<crate::ui::UiEvent>,
    ) -> Result<Vec<crate::ui::UiEvent>, crate::bridge::BridgeError> {
        use crate::world_view::ui_bind::{run_ui_with, UiQueue, UiRunError};
        let mut q = UiQueue(events.clone());
        match run_ui_with(
            &mut self.root,
            &mut q,
            bridge,
            &self.strings,
            Some(&mut self.ui),
        ) {
            Ok(f) => Ok(f.unhandled),
            Err(UiRunError::Bridge(e)) => Err(e),
            Err(UiRunError::Original(e)) => {
                let note = format!("ui: {e}");
                if !self.notes.contains(&note) {
                    eprintln!("{note}");
                    self.notes.push(note);
                }
                Ok(events)
            }
        }
    }

    fn deliver(&mut self, bridge: &mut Bridge<DumpLink>, outputs: &[Output]) -> Result<()> {
        for o in outputs.iter().filter(|o| o.consumer() == Consumer::Ui) {
            if let Err(e) = self.ui.apply_output(o, bridge.world()) {
                let note = format!("ui: {e}");
                if !self.notes.contains(&note) {
                    eprintln!("{note}");
                    self.notes.push(note);
                }
            }
            self.ui.take_sounds();
            self.ui.take_skipped();
            if let Some((d, case)) = self.ui.take_dialog_answer() {
                bridge.npc_dialog_branch(&d, case)?;
            }
        }
        Ok(())
    }
}

/// The link the dump's bridge runs on: the server thread inside the
/// walk-predicting [`PredictLink`] (`--input`'s click position).
pub type DumpLink = PredictLink<ThreadLink<Link<StepClock>>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn headless_rate_is_validated_and_clock_has_no_rounding_drift() {
        assert_eq!(
            parse_args(&args(&["--ticks", "60", "--out", "o"]))
                .unwrap()
                .tick_rate
                .hz(),
            60
        );
        for hz in ["25", "60", "1000"] {
            let a = parse_args(&args(&["--ticks", "60", "--out", "o", "--tick-rate", hz])).unwrap();
            assert_eq!(step_ms(a.tick_rate, a.tick_rate.hz()), START_MS + 1000);
            let mut driver = d2_server::host::TickDriver::new(a.tick_rate);
            assert!(!driver.poll(u64::from(START_MS), true));
            for n in 1..=a.tick_rate.hz() * 60 {
                let now = u64::from(step_ms(a.tick_rate, n));
                assert!(!driver.poll(now - 1, true));
                assert!(driver.poll(now, true), "{hz} Hz step {n}");
                assert!(!driver.poll(now, true));
            }
        }
        for hz in ["0", "24", "1001", "bad"] {
            assert!(
                parse_args(&args(&["--ticks", "60", "--out", "o", "--tick-rate", hz])).is_err()
            );
        }
        let rate = d2_sim::tick::TickRate::new(60).unwrap();
        assert_eq!(
            (1..=3)
                .map(|n| step_ms(rate, n) - step_ms(rate, n - 1))
                .collect::<Vec<_>>(),
            [16, 17, 17]
        );
    }

    #[test]
    fn gfx_ready_from_a_client_recording() {
        let text = "{\"k\":\"header\",\"format\":\"state-1\",\"tick_rate\":60}\n\
                    {\"k\":\"snap\",\"f\":3,\"units\":[],\"gfx_ready\":[[19,1],[147,6]]}\n\
                    {\"k\":\"snap\",\"f\":4,\"units\":[],\"gfx_ready\":[]}\n\
                    {\"k\":\"snap\",\"f\":5,\"units\":[],\"gfx_ready\":null}\n\
                    {\"k\":\"snap\",\"f\":6,\"units\":[]}\n";
        let m = parse_gfx_ready(text, d2_sim::tick::TickRate::GAME).unwrap();
        assert_eq!(m.keys().copied().collect::<Vec<_>>(), vec![3, 4]);
        assert!(m[&3].ready(19, 0) && !m[&3].ready(19, 1));
        assert!(m[&3].ready(147, 1) && m[&3].ready(147, 2));
        assert!(m[&4].masks.is_empty());
        assert!(parse_gfx_ready(
            "{\"k\":\"header\",\"tick_rate\":60}\n{\"k\":\"snap\",\"f\":1,\"gfx_ready\":[[1]]}",
            d2_sim::tick::TickRate::GAME
        )
        .is_err());
    }

    #[test]
    fn send_clock_file() {
        let m = parse_send_clock(
            "# send-clock 1\n# tick_rate 60\n0\t1000\n# note\n\n128\t96094\n",
            d2_sim::tick::TickRate::GAME,
        )
        .unwrap();
        assert_eq!(m.into_iter().collect::<Vec<_>>(), [(0, 1000), (128, 96094)]);
        assert!(parse_send_clock("0\t1000\n", d2_sim::tick::TickRate::GAME).is_err());
        assert!(parse_send_clock(
            "# send-clock 1\n# tick_rate 60\n5 x\n",
            d2_sim::tick::TickRate::GAME
        )
        .is_err());
        assert!(parse_send_clock(
            "# send-clock 1\n# tick_rate 60\n5\t-1\n",
            d2_sim::tick::TickRate::GAME
        )
        .is_err());
    }

    #[test]
    fn player_seed_file() {
        let m = parse_player_seed(
            "# player-seed 1\n# tick_rate 60\n30\t1047242942\t1330563431\n# n\n\n31\t1\t2\n",
            d2_sim::tick::TickRate::GAME,
        )
        .unwrap();
        assert_eq!(
            m.into_iter().collect::<Vec<_>>(),
            [(30, (1047242942, 1330563431)), (31, (1, 2))]
        );
        assert!(parse_player_seed("30\t1\t2\n", d2_sim::tick::TickRate::GAME).is_err());
        assert!(parse_player_seed(
            "# player-seed 1\n# tick_rate 60\n30\t1\n",
            d2_sim::tick::TickRate::GAME
        )
        .is_err());
        assert!(parse_player_seed(
            "# player-seed 1\n# tick_rate 60\n30\t1\t2\t3\n",
            d2_sim::tick::TickRate::GAME
        )
        .is_err());
        assert!(parse_player_seed(
            "# player-seed 1\n# tick_rate 60\n30\t-1\t2\n",
            d2_sim::tick::TickRate::GAME
        )
        .is_err());
    }

    #[test]
    fn options_parse() {
        let a = parse_args(&args(&[
            "--save",
            "S.d2s",
            "--seed",
            "1234",
            "--ticks",
            "5",
            "--every",
            "2",
            "--out",
            "o",
            "--difficulty",
            "hell",
            "--game-dir",
            "g",
            "--date",
            "2026-10-09",
            "--packets",
            "p.jsonl",
            "--save-out",
            "out.d2s",
            "--client-out",
            "c.jsonl",
            "--client-grid",
            "--no-own-c2s",
            "0x5f,3,0x5f",
            "--poke",
            "4 spawn 19 @x+3 @y+3 normal",
            "--poke",
            "4 seed-unit @1:19 0x12345678 666",
            "--input",
            "frame 10; click 600 300",
            "--send",
            "7 InteractWithEntity type=1 id=@1:148",
            "--send",
            "7 hex 2f 00 00 00 00 09 00 00 00",
            "--send-clock",
            "clock.tsv",
            "--player-seed",
            "seed.tsv",
            "--gfx-ready",
            "orig.cstate.jsonl",
        ]))
        .unwrap();
        let pokes = a.pokes.clone();
        let sends = a.sends.clone();
        assert_eq!(sends.len(), 2);
        assert_eq!(sends[0].frame, 7);
        assert_eq!(sends[0].text(), "InteractWithEntity type=1 id=@1:148");
        assert_eq!(sends[1].text(), "hex 2f 00 00 00 00 09 00 00 00");
        assert_eq!(pokes.len(), 2);
        assert_eq!(pokes[0].when, When::Frame(4));
        assert_eq!(pokes[1].op.to_string(), "seed-unit @1:19 305419896 666");
        assert_eq!(
            a,
            DumpArgs {
                save: Some("S.d2s".into()),
                seed: Some(1234),
                difficulty: 2,
                ticks: 5,
                tick_rate: d2_sim::tick::TickRate::GAME,
                every: 2,
                out: "o".into(),
                game_dir: Some("g".into()),
                date: Some("2026-10-09".into()),
                pokes,
                input: Some(input_script::parse("frame 10; click 600 300").unwrap()),
                sends,
                no_own_c2s: vec![0x5f, 0x03],
                early: Vec::new(),
                no_ping: false,
                packets: Some("p.jsonl".into()),
                rng: None,
                save_out: Some("out.d2s".into()),
                client_out: Some("c.jsonl".into()),
                client_grid: true,
                send_clock: Some("clock.tsv".into()),
                player_seed: Some("seed.tsv".into()),
                gfx_ready: Some("orig.cstate.jsonl".into()),
            }
        );
        assert!(parse_args(&args(&[
            "--ticks",
            "1",
            "--out",
            "o",
            "--poke",
            "0 time 1 0"
        ]))
        .is_err());
        assert!(parse_args(&args(&["--ticks", "1", "--out", "o", "--poke", "3 nope"])).is_err());
        for bad in ["0 Walk x=1 y=2", "3 Walk x=1", "3 Nope a=1", "3 hex 1"] {
            assert!(
                parse_args(&args(&["--ticks", "1", "--out", "o", "--send", bad])).is_err(),
                "{bad}"
            );
        }
        assert!(parse_args(&args(&["--out", "o"])).is_err(), "no --ticks");
        assert!(parse_args(&args(&["--ticks", "1"])).is_err(), "no --out");
        assert!(parse_args(&args(&["--ticks", "1", "--out", "o", "--every", "0"])).is_err());
        assert!(parse_args(&args(&["--ticks", "1", "--out", "o", "--date", "9.10.26"])).is_err());
        assert!(parse_args(&args(&["--ticks", "1", "--out", "o", "--frames", "3"])).is_err());
        // the shared input form only; `key K` is part
        // of it: a bad or missing K is refused
        for bad in [
            "click 1 2",
            "frame 2; wait 3",
            "frame 2; key zz",
            "frame 2; key",
            "frame 0",
        ] {
            assert!(
                parse_args(&args(&["--ticks", "1", "--out", "o", "--input", bad])).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn poke_record_lines() {
        use d2_sim::poke::PokeResult;
        let op = pokes::parse_poke_arg("4 spawn 19 @x+3 @y+3 normal")
            .unwrap()
            .op;
        assert_eq!(
            pokes::record_line(4, 0, &op, &PokeResult::Ok(Some(8))),
            r#"{"k":"poke","f":4,"frame":3,"i":0,"d":"spawn","r":"ok","guid":8,"src":"spawn 19 @x+3 @y+3 normal"}"#
        );
        let op = pokes::parse_poke_arg("5 seed-unit @1:19 1 2").unwrap().op;
        assert_eq!(
            pokes::record_line(5, 1, &op, &PokeResult::Unresolved("@1:19".into())),
            r#"{"k":"poke","f":5,"frame":4,"i":1,"d":"seed-unit","r":"unresolved","note":"@1:19","src":"seed-unit @1:19 1 2"}"#
        );
        let op = pokes::parse_poke_arg("6 msg 0x01 @x+2 @y").unwrap().op;
        assert_eq!(
            pokes::record_line(
                6,
                0,
                &op,
                &PokeResult::FailedWith("duplicate filter".into())
            ),
            r#"{"k":"poke","f":6,"frame":5,"i":0,"d":"msg","r":"failed","note":"duplicate filter","src":"msg 1 @x+2 @y"}"#
        );
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(11_016), "2000-02-29");
        assert_eq!(civil_date(20_735), "2026-10-09");
        assert!(is_date(&today()));
    }
}
