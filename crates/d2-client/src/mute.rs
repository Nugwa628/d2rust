//! The process mute switch: automated runs never make sound.
//!
//! `--mute` (any subcommand) or `D2RUST_MUTE=1` mutes this process: Bevy's
//! audio plugin is left out, so no output device is opened and nothing
//! pulls the game's mixed stream or the menu music. The game side runs
//! unchanged (sound requests, the mixer state, `--sound-log`), unlike
//! `play --no-sound`, which takes the sound device away from the game
//! itself. `--no-sound` mutes too (it keeps its own effect on the game as
//! well): on its own it left the audio plugin in, so the output device was
//! opened and the mixed stream played. An offscreen run (`--offscreen`) is
//! muted unless `--sound` asks for sound. The environment wins over the flags: `D2RUST_MUTE=1` mutes
//! even with `--sound`. The player's options and saves are not touched.
//!
//! A muted process sets `D2RUST_MUTE=1` for the processes it starts (a
//! sweep's or explore's game runs), so they are muted too.

use std::sync::atomic::{AtomicBool, Ordering};

use bevy::app::{PluginGroup, PluginGroupBuilder};

/// The environment switch: `1` (or `true`, `yes`, `on`) mutes.
pub const MUTE_VAR: &str = "D2RUST_MUTE";

static MUTED: AtomicBool = AtomicBool::new(false);

/// Whether a run with these arguments and this `D2RUST_MUTE` value is
/// muted (the rule in the module docs).
pub fn decide(args: &[String], env: Option<&str>) -> bool {
    if env.is_some_and(env_mutes) {
        return true;
    }
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--mute") || has("--no-sound") {
        return true;
    }
    has("--offscreen") && !has("--sound")
}

/// Whether a `D2RUST_MUTE` value asks for mute.
fn env_mutes(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Decides this process's mute from `args` and the environment, removes
/// `--mute` and `--sound` from `args` (every subcommand accepts them), and
/// marks the environment for child processes when muted. Returns the
/// decision.
pub fn init(args: &mut Vec<String>) -> bool {
    let env = crate::launch::env_os(MUTE_VAR).map(|v| v.to_string_lossy().into_owned());
    let muted = decide(args, env.as_deref());
    args.retain(|a| a != "--mute" && a != "--sound");
    set(muted);
    if muted {
        std::env::set_var(MUTE_VAR, "1");
    }
    muted
}

/// Sets this process's mute (tests and embedders; `init` for the game).
pub fn set(muted: bool) {
    MUTED.store(muted, Ordering::Relaxed);
}

/// Whether this process is muted.
pub fn muted() -> bool {
    MUTED.load(Ordering::Relaxed)
}

/// `plugins` without Bevy's audio plugin when this process is muted: no
/// output device is opened. Every window app's `DefaultPlugins` goes
/// through here.
pub fn plugins(plugins: impl PluginGroup) -> PluginGroupBuilder {
    let b = plugins.build();
    if muted() {
        b.disable::<bevy::audio::AudioPlugin>()
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn mute_flag_mutes_any_subcommand() {
        for sub in [
            "play",
            "explore",
            "soak",
            "shots",
            "verify",
            "autoplay-host",
        ] {
            assert!(decide(&args(&[sub, "--mute"]), None), "{sub}");
            assert!(!decide(&args(&[sub]), None), "{sub}");
        }
        assert!(decide(&args(&["--mute"]), None));
    }

    #[test]
    fn env_mutes_and_wins() {
        for v in ["1", "true", "YES", "on", " 1 "] {
            assert!(decide(&args(&["play"]), Some(v)), "{v}");
            assert!(decide(&args(&["play", "--sound"]), Some(v)), "{v}");
            assert!(
                decide(&args(&["play", "--offscreen", "--sound"]), Some(v)),
                "{v}"
            );
        }
        for v in ["0", "", "false", "no", "off"] {
            assert!(!decide(&args(&["play"]), Some(v)), "{v}");
        }
    }

    #[test]
    fn no_sound_is_an_alias() {
        assert!(decide(&args(&["play", "--no-sound"]), None));
        assert!(decide(&args(&["play", "--no-sound", "--sound"]), None));
        assert!(decide(
            &args(&["play", "--offscreen", "--no-sound", "--sound"]),
            None
        ));
        // `init` leaves `--no-sound` for `play`'s own parser.
        let mut a = args(&["play", "--no-sound", "--mute"]);
        set(false);
        assert!(init(&mut a));
        assert_eq!(a, args(&["play", "--no-sound"]));
    }

    #[test]
    fn offscreen_implies_mute_unless_sound() {
        assert!(decide(&args(&["play", "--offscreen"]), None));
        assert!(!decide(&args(&["play", "--offscreen", "--sound"]), None));
        assert!(decide(
            &args(&["play", "--offscreen", "--sound", "--mute"]),
            None
        ));
        assert!(!decide(&args(&["play", "--sound"]), None));
    }

    #[test]
    fn muted_plugins_leave_audio_out() {
        set(true);
        let muted = plugins(bevy::DefaultPlugins);
        set(false);
        let on = plugins(bevy::DefaultPlugins);
        assert!(!muted.enabled::<bevy::audio::AudioPlugin>());
        assert!(on.enabled::<bevy::audio::AudioPlugin>());
    }

    /// Bevy's audio plugin alone, as `DefaultPlugins` carries it.
    struct AudioOnly;

    impl PluginGroup for AudioOnly {
        fn build(self) -> PluginGroupBuilder {
            PluginGroupBuilder::start::<Self>().add(bevy::audio::AudioPlugin::default())
        }
    }

    /// A muted app never builds the audio backend: the audio plugin's
    /// resources (the output device's among them) are never created, and
    /// no audio source can be registered on it.
    #[test]
    fn muted_app_has_no_audio_backend() {
        use bevy::prelude::*;
        set(true);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .add_plugins(plugins(AudioOnly));
        app.finish();
        app.cleanup();
        app.update();
        set(false);
        assert!(app.world().get_resource::<GlobalVolume>().is_none());
        assert!(app
            .world()
            .get_resource::<Assets<bevy::audio::AudioSource>>()
            .is_none());
        assert!(app
            .world()
            .get_resource::<Assets<crate::audio::output::MixerStream>>()
            .is_none());
    }
}
