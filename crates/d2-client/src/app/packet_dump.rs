//! `d2-client state-dump... --packets FILE`: the packet recorder of `d2-server`
//! ([`d2_server::packets::PacketLog`]) installed on the dump's host at build
//! time, on the server thread, before the bridge sends anything; its
//! `packets-raw-1` lines are written after every bridge frame, between a header
//! and a footer. Recording only reads: the dump's game and its state lines are
//! the same with or without `--packets`.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use d2_server::packets::{PacketLines, PacketLog};
use d2_server::seams::Clock;
use serde_json::json;

use super::server_thread::ThreadLink;
use super::single_player::Link;
use super::state_dump::RunInfo;

/// The packets file format.
pub const FORMAT: &str = "packets-raw-1";

/// An open packets file and the recorder's lines.
pub struct PacketDump {
    lines: PacketLines,
    out: Box<dyn Write>,
}

impl PacketDump {
    /// Installs the recorder on the host behind `link` and writes the
    /// header to `out`.
    pub fn install<C: Clock + Send + 'static>(
        link: &mut ThreadLink<Link<C>>,
        mut out: Box<dyn Write>,
        info: &RunInfo,
        seed: u32,
    ) -> Result<Self> {
        let (log, lines) = PacketLog::new();
        link.with(move |l| l.host_mut().set_packet_observer(Some(Box::new(log))))?;
        let header = json!({
            "type": "header",
            "format": FORMAT,
            "side": "d2rust",
            "tool": info.tool,
            "date": info.date,
            "command": info.command,
            "save": info.save,
            "seed": seed,
            "tick_rate": link.with(|l| l.host().game.game.tick_rate.hz())?,
        });
        writeln!(out, "{header}")?;
        Ok(Self { lines, out })
    }

    /// Opens `path` (creating its folder) and installs the recorder.
    pub fn create<C: Clock + Send + 'static>(
        link: &mut ThreadLink<Link<C>>,
        path: &Path,
        info: &RunInfo,
        seed: u32,
    ) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let file =
            std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
        Self::install(link, Box::new(std::io::BufWriter::new(file)), info, seed)
    }

    /// Writes the records made since the last call.
    pub fn drain(&mut self) -> Result<()> {
        for l in self.lines.take() {
            writeln!(self.out, "{l}")?;
        }
        Ok(())
    }

    /// Writes the rest and the footer (record count, counts per type,
    /// `notes`).
    pub fn finish(mut self, notes: &[String]) -> Result<()> {
        self.drain()?;
        let footer = json!({
            "type": "footer",
            "events": self.lines.events(),
            "counts": self.lines.counts(),
            "notes": notes,
        });
        writeln!(self.out, "{footer}")?;
        self.out.flush()?;
        Ok(())
    }
}
