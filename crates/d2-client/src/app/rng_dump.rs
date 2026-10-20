//! `d2-client state-dump... --rng FILE`: every seeded draw of the dump's server
//! game (`d2_sim::debug::rng_trace`, `rng-trace` feature) as `rng-raw-1` lines
//! with frame and owner, the format `record_rng.py --frames` writes.
//!
//! The log is per thread and off by default: [`start_with`] builds the game as
//! [`single_player::start_with`] does, but turns the log on in the server
//! thread before the game is created, so game creation's draws are there (frame
//! 0). After every bridge frame [`RngDump::drain`] takes the server's draws and
//! the known seeds through the bridge ([`crate::bridge::rng_trace`]), assigns
//! frames and owners on this thread and writes the lines. The log only copies
//! values: the dump's game and its state lines are the same with or without
//! `--rng`. Built without the feature, `--rng` is an error.

use std::path::Path;

use anyhow::Result;
use d2_server::seams::Clock;

use super::server_thread::ThreadLink;
use super::single_player::{self, Character, GameData, Link, Started};
use super::state_dump::RunInfo;
use crate::bridge::Bridge;

/// [`single_player::start_with`], with the server thread's draw log on
/// from before the game is built when `traced`.
pub fn start_with<C: Clock + Send + 'static>(
    data: GameData,
    seed: u32,
    character: Character,
    clock: C,
    traced: bool,
) -> Result<(ThreadLink<Link<C>>, Started)> {
    if !traced {
        return Ok(single_player::start_with(data, seed, character, clock)?);
    }
    traced_start(data, seed, character, clock)
}

#[cfg(feature = "rng-trace")]
fn traced_start<C: Clock + Send + 'static>(
    data: GameData,
    seed: u32,
    character: Character,
    clock: C,
) -> Result<(ThreadLink<Link<C>>, Started)> {
    use crate::bridge::local::{LocalLink, PendingSession};
    use d2_server::adapters::ProtoSizes;
    use d2_server::host::Host;
    let (tx, rx) = std::sync::mpsc::channel();
    let link = ThreadLink::spawn(move || {
        d2_sim::debug::rng_trace::start();
        let g = single_player::build_with(&data, seed, character)?;
        let _ = tx.send(Started {
            prices: g.sim.world.rest.prices.clone(),
            wire: d2_proto::envelope::Wire::Original,
        });
        Ok::<_, single_player::BuildError>(LocalLink::new(Host::new(
            g.sim,
            ProtoSizes,
            PendingSession::default(),
            clock,
        )))
    })?;
    let started = rx.recv().map_err(|_| anyhow::anyhow!("game not started"))?;
    Ok((link, started))
}

#[cfg(not(feature = "rng-trace"))]
fn traced_start<C: Clock + Send + 'static>(
    _: GameData,
    _: u32,
    _: Character,
    _: C,
) -> Result<(ThreadLink<Link<C>>, Started)> {
    anyhow::bail!(NO_FEATURE)
}

#[cfg(not(feature = "rng-trace"))]
const NO_FEATURE: &str = "--rng needs d2-client built with the rng-trace feature \
                          (cargo run --release -p d2-client --features rng-trace -- state-dump ...)";

/// An open RNG file.
pub struct RngDump {
    #[cfg(feature = "rng-trace")]
    out: Box<dyn std::io::Write>,
    #[cfg(feature = "rng-trace")]
    owners: d2_sim::debug::rng_trace::Owners,
    #[cfg(feature = "rng-trace")]
    lines: u64,
}

impl RngDump {
    /// Opens `path` (creating its folder) and writes the header.
    #[cfg(feature = "rng-trace")]
    pub fn create(
        path: &Path,
        info: &RunInfo,
        seed: u32,
        rate: d2_sim::tick::TickRate,
    ) -> Result<Self> {
        use anyhow::Context;
        use std::io::Write;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let file =
            std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
        let mut out: Box<dyn Write> = Box::new(std::io::BufWriter::new(file));
        writeln!(
            out,
            "{}",
            rate_header(
                &d2_sim::debug::rng_trace::header_line(&info.tool, &info.date, &info.command, seed),
                rate,
            )?
        )?;
        Ok(Self {
            out,
            owners: d2_sim::debug::rng_trace::Owners::new(),
            lines: 0,
        })
    }

    /// Without the feature: an error naming the build to use.
    #[cfg(not(feature = "rng-trace"))]
    pub fn create(_: &Path, _: &RunInfo, _: u32, _: d2_sim::tick::TickRate) -> Result<Self> {
        anyhow::bail!(NO_FEATURE)
    }

    /// Writes the draws made since the last call (frames and owners).
    #[cfg(feature = "rng-trace")]
    pub fn drain<L>(&mut self, bridge: &mut Bridge<L>) -> Result<()>
    where
        L: crate::bridge::rng_trace::RngTraceSource,
        L::Error: std::error::Error + Send + Sync + 'static,
    {
        use std::io::Write;
        let (entries, known) = bridge.rng_trace_drain()?;
        for r in self.owners.resolve(entries, known) {
            writeln!(self.out, "{}", self.owners.line(&r))?;
            self.lines += 1;
        }
        Ok(())
    }

    #[cfg(not(feature = "rng-trace"))]
    pub fn drain<L>(&mut self, _: &mut Bridge<L>) -> Result<()> {
        Ok(())
    }

    /// Writes the footer and flushes.
    #[cfg(feature = "rng-trace")]
    pub fn finish(mut self, notes: &[String]) -> Result<()> {
        use std::io::Write;
        writeln!(
            self.out,
            "{}",
            d2_sim::debug::rng_trace::footer_line(self.lines, notes)
        )?;
        self.out.flush()?;
        Ok(())
    }

    #[cfg(not(feature = "rng-trace"))]
    pub fn finish(self, _: &[String]) -> Result<()> {
        Ok(())
    }
}

/// The server thread's draw log and seeds, read between two frames.
#[cfg(feature = "rng-trace")]
impl<C: Clock + Send + 'static> crate::bridge::rng_trace::RngTraceSource for ThreadLink<Link<C>> {
    type Error = super::server_thread::ThreadStopped;
    fn rng_trace_drain(
        &mut self,
    ) -> Result<
        (
            Vec<d2_sim::debug::rng_trace::Entry>,
            Vec<d2_sim::debug::rng_trace::Known>,
        ),
        Self::Error,
    > {
        self.with(|l| {
            let sim = &l.host().game;
            (
                d2_sim::debug::rng_trace::drain(),
                d2_sim::debug::rng_trace::known_world(&sim.game, &sim.events),
            )
        })
    }
}

#[cfg(any(feature = "rng-trace", test))]
fn rate_header(text: &str, rate: d2_sim::tick::TickRate) -> Result<String> {
    let mut value: serde_json::Value = serde_json::from_str(text)?;
    value["tick_rate"] = rate.hz().into();
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn rng_recording_states_the_selected_rate() {
        for hz in [25, 60] {
            let rate = d2_sim::tick::TickRate::new(hz).unwrap();
            let value: serde_json::Value = serde_json::from_str(
                &super::rate_header(
                    r#"{"type":"header","format":"rng-raw-1","seed":1234}"#,
                    rate,
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(value["tick_rate"], hz);
            assert_eq!(value["seed"], 1234);
        }
    }
}
