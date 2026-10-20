//! VSync and the frame limit: how frames are paced to the screen.
//!
//! **VSync** (`[video] vsync`, the Video page's VSync row) chooses the
//! window's present mode:
//! - On (the default): `Fifo`. Every frame waits for the monitor's next
//!   refresh, so a frame is never shown half-drawn (no tearing). `Fifo` is
//!   supported on every system. It is used rather than Bevy's `AutoVsync`,
//!   which picks `FifoRelaxed` where the driver offers it, and
//!   `FifoRelaxed` shows a late frame at once, mid-scan, which tears.
//! - Off: the first of `Immediate`, `Mailbox` and `Fifo` the window's
//!   surface supports ([`OFF_ORDER`]). The window asks for Bevy's
//!   `AutoNoVsync`, which the renderer resolves against the surface's
//!   supported present modes in exactly that order, so no driver-specific
//!   mode is ever forced.
//!
//! The setting applies live: [`apply_vsync`] sets the window's present mode
//! when the choice changes, and the renderer reconfigures the surface on
//! the next frame (no restart).
//!
//! Every window D2RUST opens takes its present mode from this module
//! ([`present_mode`]), and a test below fails if any other source names a
//! present mode. Each window also asks for [`FRAME_LATENCY`] queued frames
//! at most, for low input lag.
//!
//! **The frame limit** (`[video] frame_limit`) is
//! [`super::frame_limit`]'s job; [`add`] installs both.

use std::num::NonZero;

use bevy::prelude::*;
use bevy::window::{PresentMode, PrimaryWindow};

use super::config::Settings;

/// The present mode with VSync on.
pub const PRESENT_MODE_ON: PresentMode = PresentMode::Fifo;

/// The present mode the window asks for with VSync off: resolved by the
/// renderer to the first mode of [`OFF_ORDER`] the surface supports.
pub const PRESENT_MODE_OFF: PresentMode = PresentMode::AutoNoVsync;

/// VSync off: the present modes tried, in order. `Fifo` is last because it
/// is supported everywhere.
pub const OFF_ORDER: [PresentMode; 3] = [
    PresentMode::Immediate,
    PresentMode::Mailbox,
    PresentMode::Fifo,
];

/// How many frames the GPU may queue ahead of the screen, at most: two, so
/// input is at most two frames old when its frame is shown, while a frame
/// can still be drawn while the last one waits for the refresh (one would
/// halve the rate on a slow GPU whenever a frame misses the refresh).
pub const FRAME_LATENCY: NonZero<u32> = NonZero::new(2).unwrap();

/// The present mode a window asks for with VSync `vsync`.
pub fn present_mode(vsync: bool) -> PresentMode {
    if vsync {
        PRESENT_MODE_ON
    } else {
        PRESENT_MODE_OFF
    }
}

/// The present mode a window ends up with when it asks for `asked` and
/// its surface supports `supported`: VSync on is always `Fifo`; VSync off
/// is the first of [`OFF_ORDER`] that is supported (`Fifo` when the list
/// names none of them, since every surface has it).
pub fn resolve(asked: PresentMode, supported: &[PresentMode]) -> PresentMode {
    if asked != PRESENT_MODE_OFF {
        return PRESENT_MODE_ON;
    }
    OFF_ORDER
        .into_iter()
        .find(|m| supported.contains(m))
        .unwrap_or(PresentMode::Fifo)
}

/// The pacing the player chose: VSync and the frame limit, kept in step
/// with the Options page by [`super::config::apply_settings`].
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncChoice {
    pub vsync: bool,
    /// Frames per second, one of [`super::config::FRAME_LIMITS`]; 0 is Off.
    pub frame_limit: u16,
}

impl SyncChoice {
    pub fn of(s: &Settings) -> Self {
        SyncChoice {
            vsync: s.vsync,
            frame_limit: s.frame_limit,
        }
    }
}

/// Adds the pacing of `s`: the live VSync switch and the frame limiter.
pub fn add(app: &mut App, s: &Settings) {
    app.insert_resource(SyncChoice::of(s))
        .add_systems(Update, apply_vsync);
    super::frame_limit::add(app);
}

/// Sets the primary window's present mode to the chosen VSync when they
/// differ; the renderer reconfigures the surface on the next frame.
pub fn apply_vsync(
    choice: Option<Res<SyncChoice>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let Some(choice) = choice else {
        return;
    };
    let Ok(mut w) = windows.single_mut() else {
        return;
    };
    let mode = present_mode(choice.vsync);
    if w.present_mode != mode {
        w.present_mode = mode;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::config::{window_for, Settings, WindowMode};

    /// The play window the settings build has the chosen VSync and the
    /// low frame latency, whatever the window mode.
    #[test]
    fn the_play_window_takes_the_vsync_setting_in_every_mode() {
        for mode in WindowMode::ALL {
            for vsync in [true, false] {
                let s = Settings {
                    window_mode: mode,
                    vsync,
                    ..Default::default()
                };
                let w = window_for(&s);
                assert_eq!(w.present_mode, present_mode(vsync), "{mode:?} {vsync}");
                assert_eq!(w.desired_maximum_frame_latency, Some(FRAME_LATENCY));
            }
        }
        assert_eq!(
            window_for(&Settings::default()).present_mode,
            PRESENT_MODE_ON
        );
    }

    #[test]
    fn vsync_on_is_fifo_on_every_surface() {
        use PresentMode::*;
        for supported in [
            &[Fifo][..],
            &[Fifo, Immediate, Mailbox],
            &[Fifo, FifoRelaxed],
        ] {
            assert_eq!(resolve(present_mode(true), supported), Fifo);
        }
    }

    #[test]
    fn vsync_off_falls_back_immediate_then_mailbox_then_fifo() {
        use PresentMode::*;
        let off = present_mode(false);
        assert_eq!(resolve(off, &[Fifo, Mailbox, Immediate]), Immediate);
        assert_eq!(resolve(off, &[Fifo, Immediate]), Immediate);
        assert_eq!(resolve(off, &[Fifo, Mailbox]), Mailbox);
        assert_eq!(resolve(off, &[Mailbox, Fifo, FifoRelaxed]), Mailbox);
        assert_eq!(resolve(off, &[Fifo, FifoRelaxed]), Fifo);
        assert_eq!(resolve(off, &[Fifo]), Fifo);
    }

    #[test]
    fn present_mode_and_software_limit_follow_the_same_choice() {
        for refresh in [None, Some(60), Some(144)] {
            for limit in super::super::config::FRAME_LIMITS {
                for vsync in [false, true] {
                    let mode = resolve(present_mode(vsync), &OFF_ORDER);
                    let software = super::super::frame_limit::limit_in_force(vsync, limit, refresh);
                    if vsync {
                        // VSync paces; a limit below the refresh rate
                        // holds on top of it.
                        assert_eq!(mode, PresentMode::Fifo);
                        let below = limit != 0 && refresh.is_none_or(|r| u32::from(limit) < r);
                        assert_eq!(software, below.then_some(u32::from(limit)));
                    } else {
                        assert_eq!(mode, PresentMode::Immediate);
                        assert_eq!(software, (limit != 0).then_some(u32::from(limit)));
                    }
                }
            }
        }
    }

    /// The window follows the setting while the app runs: no restart.
    #[test]
    fn vsync_toggles_live() {
        let mut app = App::new();
        app.insert_resource(SyncChoice::of(&Settings::default()))
            .add_systems(Update, apply_vsync);
        let w = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let mode = |app: &App| app.world().get::<Window>(w).unwrap().present_mode;
        app.update();
        assert_eq!(mode(&app), PRESENT_MODE_ON);
        app.world_mut().resource_mut::<SyncChoice>().vsync = false;
        app.update();
        assert_eq!(mode(&app), PRESENT_MODE_OFF);
        app.world_mut().resource_mut::<SyncChoice>().vsync = true;
        app.update();
        assert_eq!(mode(&app), PRESENT_MODE_ON);
    }

    /// No code path picks a present mode on its own: outside this file,
    /// the client's sources never name a `PresentMode` variant, and a
    /// window's `present_mode` is only ever set from [`present_mode`].
    #[test]
    fn no_other_present_mode_in_the_sources() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this = src.join("app").join("vsync.rs");
        let mut bad = Vec::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read src dir") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") || path == this {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("read source");
                for (n, line) in text.lines().enumerate() {
                    let names_variant = line.contains("PresentMode::");
                    let sets_field =
                        line.contains("present_mode") && !line.contains("vsync::present_mode(");
                    if names_variant || sets_field {
                        bad.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                    }
                }
            }
        }
        assert!(
            bad.is_empty(),
            "present mode set outside app/vsync.rs:\n{}",
            bad.join("\n")
        );
    }
}
