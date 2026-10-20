//! Offscreen runs and their shots: `play --offscreen` plays the real game
//! in a hidden window and writes the presented picture as PNG files.
//!
//! An offscreen run is the normal GPU game (the player's resolution and
//! video settings, the native UI pass, the smooth camera, the frames drawn
//! between ticks), in a window that is never shown and never takes focus,
//! so a test can watch the game while someone else uses the PC. Input comes
//! from `--input` scripts, never from the real mouse or keyboard.
//!
//! The shot plan names server ticks (`--shot-at T[,T..]`, `--shot-every N`):
//! the tick on screen, the last tick the world view drew
//! ([`super::present::WorldViewState::last`]), not the newest tick the
//! client received. The first frame showing a planned tick or a later one
//! asks Bevy for a
//! screenshot of the primary window; the screenshot reads the window's
//! final image after every pass, so the PNG is exactly what a player would
//! see on that frame. Each shot is written at full size as
//! `tick-NNNNNN.png` in the shot folder. With a `phases:` display clock
//! (`super::display_clock`), `--shot-phase P` takes each shot on that
//! between-tick frame instead, as `tick-NNNNNN-fK.png` (K frames after the
//! tick's first frame).
//!
//! Shots only read the picture: they feed no input and never touch the
//! server, so a run's ticks are the same with or without them. Shots show
//! the player's game art, so they belong in a local work folder and are
//! never committed or uploaded.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

use crate::bridge::mirror::BridgeResource;

/// The window size of an offscreen run when `--size` is not given: a
/// common 16:9 monitor, so wide-screen presentation is what a run sees.
pub const DEFAULT_SIZE: (u32, u32) = (1920, 1080);

/// An offscreen run: the hidden window's size and the shots to take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offscreen {
    /// The hidden window's size in pixels (`--size WxH`), which is also
    /// the size of every shot.
    pub size: (u32, u32),
    /// The ticks to capture and where the PNGs go; `None`: no shots.
    pub shots: Option<ShotPlan>,
    /// `--display-clock`: the frame count drives the clock, so every run
    /// draws the same frames (`super::display_clock`); `None`: the wall
    /// clock.
    pub display_clock: Option<super::display_clock::DisplayClock>,
}

/// Which server ticks are captured, and the folder the PNGs go to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShotPlan {
    /// `--shot-dir DIR`, created when missing.
    pub dir: PathBuf,
    /// `--shot-at T[,T..]`: one shot at each of these ticks, ascending.
    pub at: Vec<u64>,
    /// `--shot-every N`: one shot every N ticks (N, 2N, ...).
    pub every: Option<u64>,
    /// `--shot-phase P`: how many frames after a planned tick's first
    /// frame the shot is taken. With a `phases:` display clock the first
    /// frame of a tick is phase 0, so this picks a between-tick phase; 0:
    /// the tick's first frame.
    pub delay: u64,
}

impl ShotPlan {
    /// Whether the run ends by itself once the last `--shot-at` shot is
    /// written. A plan with `--shot-every` keeps shooting, so the run
    /// needs `--frames` to end.
    pub fn ends_run(&self) -> bool {
        self.every.is_none() && !self.at.is_empty()
    }
}

/// `--size WxH`: a window size of at least 1 x 1.
pub fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let bad = || format!("--size {s}: expected WIDTHxHEIGHT, for example 1920x1080");
    let (w, h) = s.trim().split_once(['x', 'X']).ok_or_else(bad)?;
    let w: u32 = w.trim().parse().map_err(|_| bad())?;
    let h: u32 = h.trim().parse().map_err(|_| bad())?;
    if w == 0 || h == 0 {
        return Err(bad());
    }
    Ok((w, h))
}

/// `--shot-at T[,T..]`: ticks of at least 1, sorted, duplicates dropped.
pub fn parse_ticks(s: &str) -> Result<Vec<u64>, String> {
    let mut ticks = s
        .split(',')
        .map(|t| match t.trim().parse::<u64>() {
            Ok(n) if n > 0 => Ok(n),
            _ => Err(format!("--shot-at {s}: ticks are whole numbers from 1")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    ticks.sort_unstable();
    ticks.dedup();
    Ok(ticks)
}

/// The run-time side of a [`ShotPlan`]: which planned ticks are already
/// taken. Pure, so the "which frame is shot" rule is testable without a GPU.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShotClock {
    at: Vec<u64>,
    next_at: usize,
    every: Option<u64>,
    next_every: u64,
}

impl ShotClock {
    pub fn new(plan: &ShotPlan) -> Self {
        Self {
            at: plan.at.clone(),
            next_at: 0,
            every: plan.every.filter(|&n| n > 0),
            next_every: plan.every.unwrap_or(0),
        }
    }

    /// Called once per drawn frame with the frame's server tick: whether
    /// this frame is shot. A planned tick is shot on the first frame drawn
    /// at or after it; several planned ticks passed in one frame (a slow
    /// frame) give one shot.
    pub fn take(&mut self, tick: u64) -> bool {
        let mut due = false;
        while self.at.get(self.next_at).is_some_and(|&t| t <= tick) {
            self.next_at += 1;
            due = true;
        }
        if let Some(n) = self.every {
            if tick >= self.next_every {
                self.next_every = (tick / n + 1) * n;
                due = true;
            }
        }
        due
    }

    /// Every `--shot-at` tick is taken and there is no `--shot-every`.
    pub fn finished(&self) -> bool {
        self.every.is_none() && self.next_at >= self.at.len()
    }
}

/// The shooting state of an offscreen run, as a resource.
#[derive(Resource)]
pub struct Shots {
    dir: PathBuf,
    clock: ShotClock,
    ends_run: bool,
    /// The window size asked for; a shot of another size is reported.
    size: (u32, u32),
    /// Screenshots asked for whose PNG is not written yet.
    pending: Arc<AtomicUsize>,
    /// Frames after the tick's first frame each shot waits ([`ShotPlan::delay`]).
    delay: u64,
    /// A due shot still waiting: its tick and the frames left.
    waiting: Option<(u64, u64)>,
}

impl Shots {
    /// The plan's resource for a window of `size`; creates the shot folder.
    pub fn new(plan: &ShotPlan, size: (u32, u32)) -> std::io::Result<Self> {
        std::fs::create_dir_all(&plan.dir)?;
        Ok(Self {
            dir: plan.dir.clone(),
            clock: ShotClock::new(plan),
            ends_run: plan.ends_run(),
            size,
            pending: Arc::new(AtomicUsize::new(0)),
            delay: plan.delay,
            waiting: None,
        })
    }
}

/// Adds the shot system for `plan` to a play app whose window is `size`.
pub fn add(app: &mut App, plan: &ShotPlan, size: (u32, u32)) -> std::io::Result<()> {
    app.insert_resource(Shots::new(plan, size)?)
        .add_systems(Last, take_shots);
    Ok(())
}

/// Writes a captured picture to `path` as an RGB PNG; its width and
/// height.
pub fn write_png(image: &Image, path: &std::path::Path) -> Result<(u32, u32), String> {
    let img = image
        .clone()
        .try_into_dynamic()
        .map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    img.to_rgb8().save(path).map_err(|e| e.to_string())?;
    Ok((img.width(), img.height()))
}

/// Asks for a screenshot of the primary window on the first frame of each
/// planned tick; once the plan is done and every PNG is written, a plan
/// with an end closes the app.
fn take_shots(
    mut commands: Commands,
    mut shots: ResMut<Shots>,
    bridge: Option<Res<BridgeResource>>,
    view: Option<Res<super::present::WorldViewState>>,
    mut exit: MessageWriter<AppExit>,
) {
    if shots.ends_run && shots.clock.finished() && shots.waiting.is_none() {
        if shots.pending.load(Ordering::Acquire) == 0 {
            exit.write(AppExit::Success);
        }
        return;
    }
    let Some(bridge) = bridge else {
        return;
    };
    // A shot that waits for its phase is taken that many frames after the
    // tick's first frame.
    let (tick, delay) = match shots.waiting {
        Some((tick, 0)) => {
            shots.waiting = None;
            (tick, shots.delay)
        }
        Some((tick, left)) => {
            shots.waiting = Some((tick, left - 1));
            return;
        }
        None => {
            // The tick on screen; before the world view draws, the
            // client's newest tick.
            let tick = view
                .as_ref()
                .and_then(|v| v.last.as_ref())
                .map_or(bridge.0.world().server_ticks, |l| l.server_tick);
            if tick == 0 || !shots.clock.take(tick) {
                return;
            }
            if shots.delay > 0 {
                shots.waiting = Some((tick, shots.delay - 1));
                return;
            }
            (tick, 0)
        }
    };
    let name = match delay {
        0 => format!("tick-{tick:06}.png"),
        d => format!("tick-{tick:06}-f{d}.png"),
    };
    let path = shots.dir.join(name);
    let pending = shots.pending.clone();
    let size = shots.size;
    pending.fetch_add(1, Ordering::AcqRel);
    commands
        .spawn(Screenshot::primary_window())
        .observe(move |shot: On<ScreenshotCaptured>| {
            let written = write_png(&shot.image, &path);
            match written {
                Ok(got) if got != size => println!(
                    "shot: tick {tick} -> {} ({}x{}: the system sized the window, not {}x{})",
                    path.display(),
                    got.0,
                    got.1,
                    size.0,
                    size.1
                ),
                Ok(_) => println!("shot: tick {tick} -> {}", path.display()),
                Err(e) => eprintln!("shot: tick {tick}: {}: {e}", path.display()),
            }
            pending.fetch_sub(1, Ordering::AcqRel);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(at: &[u64], every: Option<u64>) -> ShotPlan {
        ShotPlan {
            dir: PathBuf::from("shots"),
            at: at.to_vec(),
            every,
            delay: 0,
        }
    }

    #[test]
    fn sizes_and_tick_lists_parse() {
        assert_eq!(parse_size("1920x1080"), Ok((1920, 1080)));
        assert_eq!(parse_size(" 1280X720 "), Ok((1280, 720)));
        assert!(parse_size("0x600").is_err());
        assert!(parse_size("1920").is_err());
        assert!(parse_size("ax600").is_err());
        assert_eq!(parse_ticks("30, 10,30,20"), Ok(vec![10, 20, 30]));
        assert!(parse_ticks("0").is_err());
        assert!(parse_ticks("5,,6").is_err());
    }

    #[test]
    fn a_planned_tick_is_shot_on_its_first_frame_only() {
        let mut c = ShotClock::new(&plan(&[3, 5], None));
        let shot: Vec<u64> = [1, 2, 3, 3, 3, 4, 5, 5, 6]
            .into_iter()
            .filter(|&t| c.take(t))
            .collect();
        assert_eq!(shot, [3, 5]);
        assert!(c.finished());
    }

    #[test]
    fn ticks_passed_in_one_slow_frame_give_one_shot() {
        let mut c = ShotClock::new(&plan(&[2, 3, 4], None));
        assert!(!c.take(1));
        assert!(c.take(4));
        assert!(c.finished());
        assert!(!c.take(5));
    }

    #[test]
    fn every_n_ticks_keeps_shooting_on_the_multiples() {
        let mut c = ShotClock::new(&plan(&[], Some(30)));
        let shot: Vec<u64> = (1..=300).filter(|&t| c.take(t)).collect();
        assert_eq!(shot, (1..=10).map(|k| k * 30).collect::<Vec<_>>());
        assert!(!c.finished());
        // A frame that skips a multiple still shoots once, then lines up again.
        let mut c = ShotClock::new(&plan(&[], Some(10)));
        assert!(c.take(25));
        assert!(!c.take(29));
        assert!(c.take(30));
    }

    #[test]
    fn only_a_plan_without_every_ends_the_run() {
        assert!(plan(&[5], None).ends_run());
        assert!(!plan(&[5], Some(10)).ends_run());
        assert!(!plan(&[], Some(10)).ends_run());
    }

    #[test]
    fn a_shot_makes_its_folder() {
        use bevy::asset::RenderAssetUsages;
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
        let root = std::env::temp_dir().join(format!("d2rust-shot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let image = Image::new_fill(
            Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[1, 2, 3, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        let path = root.join("fresh").join("a.png");
        assert_eq!(write_png(&image, &path), Ok((2, 2)));
        assert!(path.is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
