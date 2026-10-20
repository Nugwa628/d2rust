// D2RUST's own: the GPU sprite atlas across a long session.
//! Many warps in one session, across all five acts, then a walk in town:
//! the world view must hand the GPU a frame every tick, with every frame
//! it draws in the atlas pages the GPU reads.
//!
//! The frame store keeps every frame set the session loaded (each level,
//! act and monster seen), so the frames of a long session outgrow the
//! GPU atlas's fixed pages. The atlas places the frames a draw list needs
//! and starts over when they no longer fit; an atlas that only grows ran
//! out of pages after about sixteen warps and every frame after that was
//! skipped (owner reports: parts of the world stop drawing, then the
//! in-game picture freezes until a restart).
//!
//! The play client runs headless (`autoplay-host`, the user's files) with
//! the main-world half of the GPU path (the atlas, packing, one compose
//! job per tick); the test stands in for the render world by marking each
//! job composed.

use std::sync::atomic::Ordering;

use d2_client::app::autoplay_host::{Host, HostArgs};
use d2_client::app::poke;
use d2_client::bridge::BridgeResource;
use d2_client::frames::atlas::{AtlasError, PAGE_SIZE};
use d2_client::scene::ItemTag;
use d2_client::world_view::node::{ComposeJob, NodeComposed};
use d2_client::world_view::present::{WorldViewGpu, GPU_ATLAS_PAGES};
use d2_client::world_view::unit_motion::TickFrame;
use d2_client::world_view::{GpuAtlas, ViewError, WorldViewState};

/// Levels warped to, in order: every act's town and outdoor and dungeon
/// levels, the Pandemonium runs, and back. Run twice over: 44 warps.
const LEVELS: [u16; 22] = [
    1, 40, 75, 103, 109, 133, 134, 135, 136, 38, 2, 8, 21, 35, 47, 60, 83, 100, 110, 120, 128, 131,
];

/// The test atlas's pages (the app has [`GPU_ATLAS_PAGES`]).
const TEST_PAGES: u32 = 6;

struct Game {
    host: Host,
    /// The last compose job's `seq`.
    seq: u64,
}

impl Game {
    fn start() -> Self {
        let mut host = Host::start(&HostArgs {
            new: Some(("sorceress".into(), "Atlas".into())),
            seed: Some(7),
            ..Default::default()
        })
        .unwrap();
        // The GPU path's main-world half; the test is its render world.
        // A smaller page budget than the app's, so the frames this session
        // draws outgrow it and the atlas starts over several times.
        let mut gpu = WorldViewGpu::new().unwrap();
        gpu.atlas = GpuAtlas::new(TEST_PAGES).unwrap();
        host.app_mut()
            .insert_resource(gpu)
            .init_resource::<NodeComposed>();
        let mut g = Game { host, seq: 0 };
        g.step(30);
        g
    }

    /// One tick: the app's pass, then the job it handed out is composed.
    fn step(&mut self, n: usize) {
        for _ in 0..n {
            self.host.step(1);
            let w = self.host.app_mut().world();
            if let Some(job) = w.get_resource::<ComposeJob>() {
                let seq = job.seq;
                w.resource::<NodeComposed>().0.store(seq, Ordering::Release);
                self.seq = seq;
            }
        }
    }

    /// A server poke (as `play --poke`), between two passes.
    fn poke(&mut self, line: &str) {
        let toks: Vec<&str> = line.split_whitespace().collect();
        let op = d2_sim::poke::parse_op(&toks).unwrap();
        let code = self
            .host
            .server()
            .lock()
            .unwrap()
            .with(move |l| poke::apply_on_link(l, &op).code().to_owned())
            .unwrap();
        assert_eq!(code, "ok", "{line}");
    }

    fn client_level(&mut self) -> Option<u16> {
        self.host
            .app_mut()
            .world()
            .resource::<BridgeResource>()
            .0
            .world()
            .player_level()
    }

    fn player_at(&mut self) -> (i32, i32) {
        let p = self
            .host
            .app_mut()
            .world()
            .resource::<BridgeResource>()
            .0
            .world()
            .local()
            .and_then(|u| u.position)
            .expect("local player position");
        (i32::from(p.0), i32::from(p.1))
    }

    fn skipped(&mut self) -> u64 {
        self.host
            .app_mut()
            .world()
            .resource::<WorldViewState>()
            .skipped_frames
    }

    /// Warps to `level` and steps until the client is there.
    fn warp(&mut self, level: u16, at: &str) {
        self.poke(&format!("warp {level}"));
        let mut n = 0;
        while self.client_level() != Some(level) {
            self.step(1);
            n += 1;
            assert!(n < 400, "{at}: the client reaches level {level}");
        }
    }

    /// Steps `n` ticks; every tick hands the GPU a new job and no frame
    /// is skipped.
    fn presents(&mut self, n: usize, at: &str) {
        for _ in 0..n {
            let before = self.seq;
            self.step(1);
            assert!(self.seq > before, "{at}: a compose job every tick");
        }
        assert_eq!(self.skipped(), 0, "{at}: no frame skipped");
    }

    /// The last tick frame draws map tiles, and every frame it draws is in
    /// the pages its job handed the GPU, pixel for pixel.
    fn every_frame_resident(&mut self, at: &str) {
        let w = self.host.app_mut().world();
        let tick = w.resource::<TickFrame>();
        let job = w.resource::<ComposeJob>();
        assert_eq!(tick.seq, job.seq, "{at}: the tick frame is the last job's");
        let tiles = tick
            .items
            .iter()
            .filter(|i| matches!(i.tag, ItemTag::Tile { .. }))
            .count();
        assert!(tiles > 0, "{at}: map tiles drawn");
        let state = w.resource::<WorldViewState>();
        let slots = w.resource::<WorldViewGpu>().atlas.slots();
        for item in tick.items.iter() {
            let frame = state.assets.frames.frame(item.frame).unwrap();
            let slot = slots
                .get(item.frame.0 as usize)
                .copied()
                .flatten()
                .unwrap_or_else(|| panic!("{at}: frame {:?} not in the atlas", item.frame));
            if slot.is_empty() {
                assert!(frame.is_empty(), "{at}");
                continue;
            }
            assert_eq!((slot.w, slot.h), (frame.width, frame.height), "{at}");
            let page = &job.pages[slot.page as usize].pixels;
            for row in 0..slot.h {
                let src = (row * slot.w) as usize;
                let dst = ((slot.y + row) * PAGE_SIZE + slot.x) as usize;
                assert!(
                    page[dst..dst + slot.w as usize] == frame.pixels[src..src + slot.w as usize],
                    "{at}: frame {:?} row {row} differs on page {}",
                    item.frame,
                    slot.page
                );
            }
        }
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn forty_four_warps_and_a_town_walk_present_every_frame() {
    let mut g = Game::start();
    g.presents(10, "start");
    for (n, &level) in LEVELS.iter().chain(LEVELS.iter()).enumerate() {
        let at = format!("warp {} (level {level})", n + 1);
        g.warp(level, &at);
        g.presents(40, &at);
        g.every_frame_resident(&at);
    }
    // Back in town, walking: new frames (the walk, the town's other
    // corners) keep being placed after the long session.
    g.warp(1, "town");
    g.presents(20, "town");
    let start = g.player_at();
    for (i, (dx, dy)) in [(0, 30), (30, 0), (0, -30), (-30, 0)]
        .into_iter()
        .enumerate()
    {
        let (x, y) = g.player_at();
        // C->S 0x03: run to a point, as a click on the floor sends.
        let mut m = vec![0x03];
        m.extend_from_slice(&((x + dx) as u16).to_le_bytes());
        m.extend_from_slice(&((y + dy) as u16).to_le_bytes());
        g.host
            .app_mut()
            .world_mut()
            .resource_mut::<BridgeResource>()
            .0
            .send_bytes(&m)
            .unwrap();
        let at = format!("town walk {i}");
        g.presents(60, &at);
        g.every_frame_resident(&at);
    }
    assert_ne!(g.player_at(), start, "the player walked");
    let restarts = g
        .host
        .app_mut()
        .world()
        .resource::<WorldViewGpu>()
        .atlas
        .restarts();
    assert!(restarts > 0, "the atlas started over");
    // The session went past the point where an atlas that only grows
    // stopped: the frame store's frames together do not fit the pages.
    let store = &g
        .host
        .app_mut()
        .world()
        .resource::<WorldViewState>()
        .assets
        .frames;
    let mut whole = GpuAtlas::new(GPU_ATLAS_PAGES).unwrap();
    assert!(
        matches!(
            whole.ensure(store),
            Err(ViewError::Atlas(AtlasError::Full { .. }))
        ),
        "the session's {} frames fit {GPU_ATLAS_PAGES} pages",
        store.len()
    );
}
