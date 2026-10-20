//! The info overlay (`info`), the log ring the bug report reads, and the
//! bug report (`bugreport [note]`). Client-side only: they read the
//! client model and the window, and write files under the save folder.
//! The bug report is a `d2_report` bundle, the same format the launcher
//! shows the player and the report service accepts.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use bevy::log::tracing_subscriber::layer::{Context, Layer};
use bevy::log::tracing_subscriber::registry::Registry;
use bevy::log::BoxedLayer;
use bevy::prelude::*;

use super::server::DevCatalog;
use crate::bridge::BridgeResource;
use crate::world_view::present::WorldViewState;

/// Log lines kept for a bug report.
pub const LOG_LINES: usize = 200;

fn ring() -> &'static Mutex<VecDeque<String>> {
    static RING: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    RING.get_or_init(|| Mutex::new(VecDeque::with_capacity(LOG_LINES)))
}

/// Adds one line to the ring (the oldest goes past [`LOG_LINES`]).
pub fn push_log(line: String) {
    if let Ok(mut r) = ring().lock() {
        if r.len() == LOG_LINES {
            r.pop_front();
        }
        r.push_back(line);
    }
}

/// The ring's lines, oldest first.
pub fn last_log() -> Vec<String> {
    ring()
        .lock()
        .map(|r| r.iter().cloned().collect())
        .unwrap_or_default()
}

struct Fields(String);

impl bevy::log::tracing::field::Visit for Fields {
    fn record_debug(
        &mut self,
        field: &bevy::log::tracing::field::Field,
        value: &dyn std::fmt::Debug,
    ) {
        use std::fmt::Write;
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }
}

/// The tracing layer that fills the ring.
struct RingLayer;

impl Layer<Registry> for RingLayer {
    fn on_event(&self, event: &bevy::log::tracing::Event<'_>, _: Context<'_, Registry>) {
        let m = event.metadata();
        let mut f = Fields(String::new());
        event.record(&mut f);
        push_log(format!("{} {}: {}", m.level(), m.target(), f.0));
    }
}

/// `LogPlugin::custom_layer` of the interactive game with the tools on.
pub fn log_layer(_: &mut App) -> Option<BoxedLayer> {
    Some(Box::new(RingLayer))
}

/// The build's commit (set by the CI build), else "local build".
pub fn build_commit() -> &'static str {
    option_env!("GITHUB_SHA").unwrap_or("local build")
}

/// What the tools know of the game they run in (set by `play::run`).
#[derive(Resource, Clone, Debug)]
pub struct DevContext {
    pub seed: u32,
    pub difficulty: u8,
    /// The character's save file (none: a save cannot be made).
    pub save_path: Option<PathBuf>,
    /// The folder the tools write into (slots, bug reports).
    pub dir: PathBuf,
    pub catalog: Arc<DevCatalog>,
}

/// The info overlay's switch.
#[derive(Resource, Default)]
pub struct InfoOverlay {
    pub on: bool,
}

#[derive(Component)]
pub(super) struct InfoText;

/// A level id with its name.
pub fn level_name(cat: &DevCatalog, level: u32) -> String {
    cat.levels.iter().find(|(id, _)| *id == level).map_or_else(
        || format!("level {level}"),
        |(_, n)| format!("{n} ({level})"),
    )
}

/// The state lines the info overlay and the bug report show.
pub fn state_lines(
    b: &BridgeResource,
    view: Option<&WorldViewState>,
    ctx: &DevContext,
    fps: Option<f64>,
) -> Vec<String> {
    let w = b.0.world();
    let mut out = Vec::new();
    match (w.local(), w.local_room()) {
        (Some(u), Some(r)) => {
            let (x, y) = u.cell();
            out.push(format!(
                "area {}",
                level_name(&ctx.catalog, u32::from(r.level))
            ));
            out.push(format!("position ({x}, {y})  mode {}", u.mode));
            if let (Some(l), Some(m)) = (u.stats.get(&6), u.stats.get(&7)) {
                out.push(format!("life {}/{}", l >> 8, m >> 8));
            }
        }
        _ => out.push("not in game".into()),
    }
    out.push(format!(
        "server tick {}  frame {}  seed {}  difficulty {}",
        w.server_ticks, w.frames, ctx.seed, ctx.difficulty
    ));
    if let Some(f) = fps {
        out.push(format!("fps {f:.0}"));
    }
    if let Some(k) = view.and_then(|v| v.prev_hover) {
        let line = match w.units.get(&k) {
            Some(u) => {
                let name = if k.unit_type == 1 {
                    ctx.catalog
                        .monsters
                        .iter()
                        .find(|(id, _)| *id == u.class)
                        .map_or("", |(_, n)| n.as_str())
                } else {
                    ""
                };
                let life = u
                    .stats
                    .get(&6)
                    .map_or(String::new(), |l| format!(" life {}", l >> 8));
                format!(
                    "hover: type {} guid {} class {} {name} mode {}{life}",
                    k.unit_type, k.guid, u.class, u.mode
                )
            }
            None => format!("hover: type {} guid {}", k.unit_type, k.guid),
        };
        out.push(line);
    }
    out
}

pub(super) fn spawn_info(mut commands: Commands) {
    commands.spawn((
        InfoText,
        crate::app::session::GameEntity,
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(12.0),
            ..default()
        },
        TextColor(Color::srgb(1.0, 1.0, 0.6)),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(2.0),
            right: Val::Px(2.0),
            padding: UiRect::all(Val::Px(3.0)),
            ..default()
        },
        GlobalZIndex(1990),
        Visibility::Hidden,
    ));
}

pub(super) fn draw_info(
    info: Res<InfoOverlay>,
    ctx: Option<Res<DevContext>>,
    bridge: Option<Res<BridgeResource>>,
    view: Option<Res<WorldViewState>>,
    time: Res<Time>,
    mut fps: Local<f64>,
    mut text: Query<(&mut Text, &mut Visibility), With<InfoText>>,
) {
    let dt = time.delta_secs_f64();
    if dt > 0.0 {
        *fps = if *fps == 0.0 {
            1.0 / dt
        } else {
            *fps * 0.95 + 0.05 / dt
        };
    }
    for (mut t, mut v) in &mut text {
        let want = if info.on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
        let (Some(ctx), Some(b), true) = (ctx.as_deref(), bridge.as_deref(), info.on) else {
            continue;
        };
        let s = state_lines(b, view.as_deref(), ctx, Some(*fps)).join("\n");
        if t.0 != s {
            t.0 = s;
        }
    }
}

/// A folder name that is free under `dir`: `<secs>`, else `<secs>-<n>`.
fn fresh_dir(dir: &Path, secs: u64) -> PathBuf {
    let mut p = dir.join(secs.to_string());
    let mut n = 1;
    while p.exists() {
        p = dir.join(format!("{secs}-{n}"));
        n += 1;
    }
    p
}

/// The state dump of a bug report: the build, the character's save name
/// (the file name only, so the player's folders stay out of the report) and
/// the overlay's state lines.
pub fn report_state(state: &[String], character: Option<&Path>) -> Vec<String> {
    let mut out = vec![format!("build {}", build_commit())];
    if let Some(name) = character.and_then(Path::file_name) {
        out.push(format!("character {}", name.to_string_lossy()));
    }
    out.extend(state.iter().cloned());
    out
}

/// `bugreport [note]`: writes a bug report bundle (`d2_report`, format v1)
/// into a fresh folder under `bug-reports` and returns its path. The note,
/// the log tail and the state dump are written at once, after the player's
/// home folder and account name are replaced with `[home]` and `[user]`
/// (`d2_report::Redactor`; the launcher redacts again before sending). The
/// screenshot is added when the frame is captured, a frame later.
pub fn bug_report(
    commands: &mut Commands,
    ctx: &DevContext,
    bridge: &BridgeResource,
    view: Option<&WorldViewState>,
    note: &str,
) -> std::io::Result<PathBuf> {
    use d2_report::{Bundle, FileKind, Meta};
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let dir = fresh_dir(&ctx.dir.join("bug-reports"), secs);
    let state = report_state(
        &state_lines(bridge, view, ctx, None),
        ctx.save_path.as_deref(),
    );
    Bundle::new(Meta {
        game_version: crate::launch::VERSION.into(),
        launcher_version: None,
        os: std::env::consts::OS.into(),
        time_unix: secs,
        note: d2_report::clip(note.trim(), d2_report::MAX_NOTE_CHARS),
    })
    .and_then(|b| b.with_file(FileKind::Log, d2_report::log_tail(&last_log())))
    .and_then(|b| b.with_file(FileKind::State, d2_report::state_text(&state)))
    .and_then(|b| d2_report::Redactor::from_env().redact_bundle(b))
    .and_then(|b| b.write_dir(&dir))
    .map_err(std::io::Error::other)?;
    let shot_dir = dir.clone();
    commands
        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
        .observe(
            move |shot: On<bevy::render::view::screenshot::ScreenshotCaptured>| {
                let added = shot
                    .image
                    .clone()
                    .try_into_dynamic()
                    .map_err(|e| e.to_string())
                    .and_then(|img| {
                        let png = d2_report::screenshot_png(&img).map_err(|e| e.to_string())?;
                        Bundle::add_to_dir(&shot_dir, FileKind::Screenshot, png)
                            .map_err(|e| e.to_string())
                    });
                if let Err(e) = added {
                    warn!("bug report: no screenshot: {e}");
                }
            },
        );
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_keeps_the_last_lines_and_the_report_names_the_build() {
        for i in 0..(LOG_LINES + 5) {
            push_log(format!("line {i}"));
        }
        let l = last_log();
        assert_eq!(l.len(), LOG_LINES);
        assert_eq!(l.last().unwrap(), &format!("line {}", LOG_LINES + 4));
        let save = Path::new("/home/someone/saves/Hero.d2s");
        let t = report_state(&["area Cold Plains (3)".into()], Some(save));
        assert!(t[0].starts_with("build "));
        assert_eq!(t[1..], ["character Hero.d2s", "area Cold Plains (3)"]);
        let dump = d2_report::StateDump::parse(&d2_report::state_text(&t)).unwrap();
        assert_eq!(dump.lines, t);
        let d = std::env::temp_dir().join(format!("d2rust-fresh-{}", std::process::id()));
        std::fs::create_dir_all(d.join("7")).unwrap();
        assert_eq!(fresh_dir(&d, 7), d.join("7-1"));
        assert_eq!(fresh_dir(&d, 8), d.join("8"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
