//! Starting the game: where the game files are, the logs, and how a
//! failed run is reported (`docs/PLAYTEST.md`).
//!
//! [`game_dir`] picks the install folder: `--game-dir`, then
//! `$D2_GAME_DIR`, then the folder of `D2RUST-Game.exe` or the current
//! folder when it holds `d2data.mpq`, so the exe copied into the Diablo II
//! folder starts on a double-click.
//!
//! Logs go in `$D2RUST_LOG_DIR` when set, otherwise in the player's
//! D2RUST folder, `Documents\D2RUST\logs`:
//! `game.log` holds this run's output ([`say!`], and Bevy's log through
//! [`log_layer`]), `game.prev.log` the run before, and `crash.log` the last
//! crash or fatal error ([`install_crash_log`], [`write_error`]) with the
//! backtrace and the last [`note`]s. The shipped exe has no console, so a
//! fatal error also shows a message box ([`show_fatal`]).
// D2RUST's own (a launcher convenience).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// The game's full version: the crate's 0.1.1 plus the build number
/// (`build.rs`, `$D2RUST_BUILD`), e.g. `0.1.1.7`.
pub const VERSION: &str = env!("D2RUST_VERSION");

/// The exit codes, for the launcher: it shows the reason from the error
/// file (`--error-file`) or the crash log.
pub mod exit {
    /// The game ended normally.
    pub const OK: u8 = 0;
    /// A fatal error.
    pub const ERROR: u8 = 1;
    /// The command line was wrong.
    pub const USAGE: u8 = 2;
    /// The Diablo II files were not found.
    pub const NO_GAME_FILES: u8 = 3;
    /// A panic (Rust's own code; the crash log holds the backtrace).
    pub const PANIC: u8 = 101;
}

/// A wrong command line (exit code [`exit::USAGE`]).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Usage(pub String);

/// Where the one-line reason for a failed run goes (`--error-file`).
static ERROR_FILE: OnceLock<PathBuf> = OnceLock::new();

/// The archive whose presence marks a Diablo II install folder.
pub const MARKER: &str = "d2data.mpq";

/// The crash log's file name in the logs folder.
pub const CRASH_LOG: &str = "crash.log";

/// This run's log, in the logs folder.
pub const GAME_LOG: &str = "game.log";

/// The previous run's log, kept for a bug report.
pub const PREV_LOG: &str = "game.prev.log";

/// The crash log's name next to the exe, when the logs folder cannot be
/// made.
const FALLBACK_CRASH_LOG: &str = "D2RUST-crash.log";

/// The environment variable `name` (a `D2RUST_` name), else the same name
/// with the `D2RS_` prefix earlier builds read (legacy-name), so launchers,
/// scripts and players that still set the old name keep working.
pub fn env_os(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).or_else(|| {
        let rest = name.strip_prefix("D2RUST_")?;
        std::env::var_os(format!("D2RS_{rest}"))
    })
}

/// [`env_os`] as text; `None` when unset or not Unicode.
pub fn env(name: &str) -> Option<String> {
    env_os(name).and_then(|v| v.into_string().ok())
}

/// Set to anything: never show a message box (CI, scripts).
pub const NO_DIALOG_VAR: &str = "D2RUST_NO_DIALOG";

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// The notes kept for the crash log (oldest dropped first).
const NOTES: usize = 16;

static LAST: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The install folder, in order: `arg` (`--game-dir`), `env`
/// (`$D2_GAME_DIR`, when not empty), then the first of `candidates` that
/// holds [`MARKER`]. `None`: none found.
pub fn game_dir(
    arg: Option<&Path>,
    env: Option<PathBuf>,
    candidates: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(a) = arg {
        return Some(a.to_path_buf());
    }
    if let Some(e) = env.filter(|e| !e.as_os_str().is_empty()) {
        return Some(e);
    }
    candidates.iter().find(|c| holds_marker(c)).cloned()
}

/// The folders searched when nothing names the install: the exe's
/// folder, then the current folder.
pub fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = exe_dir() {
        v.push(d);
    }
    if let Ok(d) = std::env::current_dir() {
        if !v.contains(&d) {
            v.push(d);
        }
    }
    v
}

/// Whether `dir` holds [`MARKER`] (any letter case: installs differ).
fn holds_marker(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(MARKER))
    })
}

/// The folder of the running exe.
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

/// Overrides the logs folder without changing saves or settings.
pub const LOG_DIR_VAR: &str = "D2RUST_LOG_DIR";

/// The logs folder (`$D2RUST_LOG_DIR`, else `Documents\D2RUST\logs`),
/// made if missing. An empty override uses the default. `None`: the
/// chosen folder cannot be made (no retry in the player's folder).
pub fn logs_dir() -> Option<PathBuf> {
    let dir = env_os(LOG_DIR_VAR)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(crate::user_dir::logs);
    std::fs::create_dir_all(&dir).ok().map(|_| dir)
}

/// Where the crash log goes: the logs folder, else next to the exe.
pub fn crash_log_path() -> PathBuf {
    logs_dir().map_or_else(
        || exe_dir().unwrap_or_default().join(FALLBACK_CRASH_LOG),
        |d| d.join(CRASH_LOG),
    )
}

/// Starts this run's `game.log`, keeping the last one as `game.prev.log`.
/// Returns its path; `None` when the logs folder cannot be written (the
/// game runs on, logging to stdout only).
pub fn open_log() -> Option<PathBuf> {
    let dir = logs_dir()?;
    let path = dir.join(GAME_LOG);
    if path.exists() {
        let _ = std::fs::rename(&path, dir.join(PREV_LOG));
    }
    let file = std::fs::File::create(&path).ok()?;
    *LOG.lock().ok()? = Some(file);
    Some(path)
}

/// Appends one line to `game.log` (when it is open).
pub fn log_line(line: &str) {
    use std::io::Write;
    if let Ok(mut log) = LOG.lock() {
        if let Some(f) = log.as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// One line on stdout and in `game.log` ([`say!`]).
pub fn say(line: &str) {
    println!("{line}");
    log_line(line);
}

/// One line on stderr and in `game.log` ([`say_err!`]).
pub fn say_err(line: &str) {
    eprintln!("{line}");
    log_line(line);
}

/// `println!` that also goes to `game.log`.
#[macro_export]
macro_rules! say {
    ($($t:tt)*) => { $crate::launch::say(&format!($($t)*)) };
}

/// `eprintln!` that also goes to `game.log`.
#[macro_export]
macro_rules! say_err {
    ($($t:tt)*) => { $crate::launch::say_err(&format!($($t)*)) };
}

/// Bevy's log (`info!`, `warn!`, …) copied into `game.log`, plus the dev
/// tools' log ring when they are on: every app's `LogPlugin` gets it
/// through [`log_plugin`].
pub fn log_layer(app: &mut bevy::app::App) -> Option<bevy::log::BoxedLayer> {
    let mut layers: Vec<bevy::log::BoxedLayer> = vec![Box::new(FileLayer)];
    if DEV_LOG.load(std::sync::atomic::Ordering::Relaxed) {
        layers.extend(crate::app::dev::info::log_layer(app));
    }
    Some(Box::new(layers))
}

/// Whether [`log_layer`] adds the dev log ring (the last [`log_plugin`]).
static DEV_LOG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The `LogPlugin` every app of this run uses: Bevy's defaults with
/// [`log_layer`]; `dev` also keeps the log ring the bug report reads.
pub fn log_plugin(dev: bool) -> bevy::log::LogPlugin {
    DEV_LOG.store(dev, std::sync::atomic::Ordering::Relaxed);
    bevy::log::LogPlugin {
        custom_layer: log_layer,
        ..Default::default()
    }
}

struct FileLayer;

impl bevy::log::tracing_subscriber::Layer<bevy::log::tracing_subscriber::registry::Registry>
    for FileLayer
{
    fn on_event(
        &self,
        event: &bevy::log::tracing::Event<'_>,
        _: bevy::log::tracing_subscriber::layer::Context<
            '_,
            bevy::log::tracing_subscriber::registry::Registry,
        >,
    ) {
        let m = event.metadata();
        let mut text = Fields(String::new());
        event.record(&mut text);
        log_line(&format!("{} {}: {}", m.level(), m.target(), text.0));
    }
}

/// An event's fields as `message key=value ...`.
struct Fields(String);

impl bevy::log::tracing::field::Visit for Fields {
    fn record_debug(
        &mut self,
        field: &bevy::log::tracing::field::Field,
        value: &dyn std::fmt::Debug,
    ) {
        use std::fmt::Write;
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, "{}={value:?}", field.name());
        }
    }
}

/// Whether a Bevy system error must end the process: only the fatal
/// (`Panic`) severity. The server runs on its own thread, so a system error
/// there means the frame cannot go on.
pub fn system_error_is_fatal(severity: bevy::ecs::error::Severity) -> bool {
    matches!(severity, bevy::ecs::error::Severity::Panic)
}

/// The play apps' Bevy fallback error handler: a fatal system error (a
/// panicking server/compute task, or a system returning a panic-severity
/// error) writes the crash log and exits non-zero instead of leaving the
/// game running with a dead server. Other severities keep Bevy's logging.
pub fn exit_on_system_error(
    error: bevy::ecs::error::BevyError,
    ctx: bevy::ecs::error::ErrorContext,
) {
    if system_error_is_fatal(error.severity()) {
        let reason = format!("{ctx}: {error}");
        write_error(&anyhow::anyhow!("{reason}"));
        std::process::exit(i32::from(exit::PANIC));
    }
    bevy::ecs::error::match_severity(error, ctx);
}

/// Whether a fatal error may show a message box: not when the game runs
/// in a console (the message is on screen there), not when the launcher
/// asked for the reason in a file (`--error-file`: it shows the reason
/// itself), not in an offscreen run (`play --offscreen` never puts a
/// window in front of the person using the PC) and not when
/// `D2RUST_NO_DIALOG` is set.
pub fn dialogs_allowed() -> bool {
    use std::io::IsTerminal;
    env_os(NO_DIALOG_VAR).is_none()
        && ERROR_FILE.get().is_none()
        && !std::io::stderr().is_terminal()
        && !std::env::args().any(|a| a == "--offscreen")
}

/// The message box text for a fatal error: the reason, then where the
/// details are.
pub fn fatal_text(reason: &str, log: &Path) -> String {
    format!(
        "D2RUST stopped:\n\n{reason}\n\nThe details are in {}",
        log.display()
    )
}

/// Shows a fatal error in a message box (Windows; see [`dialogs_allowed`]).
pub fn show_fatal(reason: &str) {
    if dialogs_allowed() {
        message_box(&fatal_text(reason, &crash_log_path()));
    }
}

#[cfg(windows)]
fn message_box(text: &str) {
    let _ = win_msgbox::error::<win_msgbox::Okay>(text)
        .title("D2RUST")
        .show();
}

#[cfg(not(windows))]
fn message_box(_: &str) {}

/// Takes `--error-file <path>` out of `args`: the file a failed run writes
/// its one-line reason to, so the launcher can show it. Removed so the
/// commands never see it.
pub fn take_error_file(args: &mut Vec<String>) -> Option<PathBuf> {
    let i = args.iter().position(|a| a == "--error-file")?;
    let path = args.get(i + 1).map(PathBuf::from);
    args.drain(i..(i + 2).min(args.len()));
    path
}

/// Sets the error file for this run, and removes one an earlier run left.
pub fn set_error_file(path: PathBuf) {
    let _ = std::fs::remove_file(&path);
    let _ = ERROR_FILE.set(path);
}

/// Writes `reason` to the error file, when one was given.
fn write_error_file(reason: &str) {
    if let Some(p) = ERROR_FILE.get() {
        let _ = std::fs::write(p, format!("{reason}\n"));
    }
}

/// The exit code for the error `main` returns: a wrong command line, no
/// game files, or any other fatal error.
pub fn exit_code(error: &anyhow::Error) -> u8 {
    use crate::app::single_player::BuildError;
    let chain = || error.chain();
    if chain().any(|e| e.downcast_ref::<Usage>().is_some()) {
        exit::USAGE
    } else if chain().any(|e| {
        matches!(e.downcast_ref::<BuildError>(), Some(BuildError::NoGameDir))
            || e.to_string().starts_with("no game files")
    }) {
        exit::NO_GAME_FILES
    } else {
        exit::ERROR
    }
}

/// Remembers one line of "what the client was doing" for the crash log.
pub fn note(line: impl Into<String>) {
    if let Ok(mut l) = LAST.lock() {
        if l.len() == NOTES {
            l.remove(0);
        }
        l.push(line.into());
    }
}

/// The crash log's text: the headline, the notes, then `detail`.
pub fn report(headline: &str, notes: &[String], detail: &str) -> String {
    let mut s = format!(
        "D2RUST {} ({} {})\n{headline}\n\nargs: {:?}\n\nlast state (oldest first):\n",
        VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::args().collect::<Vec<_>>(),
    );
    if notes.is_empty() {
        s.push_str("  (none)\n");
    }
    for n in notes {
        s.push_str("  ");
        s.push_str(n);
        s.push('\n');
    }
    s.push('\n');
    s.push_str(detail);
    s.push('\n');
    s
}

fn notes() -> Vec<String> {
    LAST.lock().map(|l| l.clone()).unwrap_or_default()
}

fn write_log(text: &str) -> Option<PathBuf> {
    let path = crash_log_path();
    std::fs::write(&path, text).ok().map(|_| path)
}

/// Installs the panic hook: the default message on stderr as before, then
/// the crash log with the panic, the notes and a backtrace, and the
/// message box.
pub fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let thread = std::thread::current();
        let headline = format!(
            "panic in thread {}: {info}",
            thread.name().unwrap_or("<unnamed>")
        );
        write_error_file(&headline);
        log_line(&headline);
        let backtrace = std::backtrace::Backtrace::force_capture();
        let text = report(&headline, &notes(), &format!("backtrace:\n{backtrace}"));
        if let Some(p) = write_log(&text) {
            say_err(&format!("crash log written to {}", p.display()));
        }
        show_fatal(&headline);
    }));
}

/// Writes a fatal error (the `Err` `main` returns) to the crash log.
pub fn write_error(error: &anyhow::Error) {
    let reason = format!("{error:#}");
    write_error_file(&reason);
    let text = report(&format!("error: {reason}"), &notes(), "");
    if let Some(p) = write_log(&text) {
        say_err(&format!("error log written to {}", p.display()));
    }
    show_fatal(&reason);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("d2rust-launch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_old_variable_name_is_still_read() {
        // Names no other test or tool sets.
        std::env::set_var("D2RS_LAUNCH_TEST_OLD", "old");
        assert_eq!(env("D2RUST_LAUNCH_TEST_OLD").as_deref(), Some("old"));
        std::env::set_var("D2RUST_LAUNCH_TEST_BOTH", "new");
        std::env::set_var("D2RS_LAUNCH_TEST_BOTH", "old");
        assert_eq!(env("D2RUST_LAUNCH_TEST_BOTH").as_deref(), Some("new"));
        assert_eq!(env("D2RUST_LAUNCH_TEST_NONE"), None);
    }

    #[test]
    fn game_dir_order() {
        let empty = scratch("empty");
        let install = scratch("install");
        std::fs::write(install.join("D2DATA.MPQ"), b"").unwrap();
        let c = [empty.clone(), install.clone()];
        let arg = Path::new("arg");
        assert_eq!(
            game_dir(Some(arg), Some("env".into()), &c),
            Some(PathBuf::from("arg"))
        );
        assert_eq!(game_dir(None, Some("env".into()), &c), Some("env".into()));
        // An empty variable is unset; the marker is matched in any case.
        assert_eq!(game_dir(None, Some("".into()), &c), Some(install.clone()));
        assert_eq!(game_dir(None, None, &c[..1]), None);
        let _ = std::fs::remove_dir_all(empty);
        let _ = std::fs::remove_dir_all(install);
    }

    #[test]
    fn only_a_panic_severity_system_error_ends_the_process() {
        use bevy::ecs::error::Severity;
        assert!(system_error_is_fatal(Severity::Panic));
        for severity in [
            Severity::Ignore,
            Severity::Trace,
            Severity::Debug,
            Severity::Info,
            Severity::Warning,
            Severity::Error,
        ] {
            assert!(!system_error_is_fatal(severity));
        }
    }

    #[test]
    fn report_holds_notes_and_detail() {
        let r = report("panic: x", &["play: new sorceress".into()], "backtrace:\nf");
        assert!(r.contains("panic: x"));
        assert!(r.contains("  play: new sorceress\n"));
        assert!(r.ends_with("backtrace:\nf\n"));
        assert!(report("e", &[], "").contains("(none)"));
    }

    #[test]
    fn the_error_file_flag_is_taken_out() {
        let mut a: Vec<String> = ["play", "--error-file", "e.txt", "--seed", "4"]
            .map(String::from)
            .into();
        assert_eq!(take_error_file(&mut a), Some(PathBuf::from("e.txt")));
        assert_eq!(a, ["play", "--seed", "4"]);
        let mut b: Vec<String> = vec!["play".into()];
        assert_eq!(take_error_file(&mut b), None);
        assert_eq!(b, ["play"]);
    }

    #[test]
    fn exit_codes_by_error() {
        use crate::app::single_player::BuildError;
        assert_eq!(exit_code(&Usage("usage: x".into()).into()), exit::USAGE);
        let no_files = anyhow::Error::from(BuildError::NoGameDir).context("play");
        assert_eq!(exit_code(&no_files), exit::NO_GAME_FILES);
        assert_eq!(
            exit_code(&anyhow::anyhow!("no game files: x")),
            exit::NO_GAME_FILES
        );
        assert_eq!(exit_code(&anyhow::anyhow!("bad save")), exit::ERROR);
        assert!(VERSION.starts_with(env!("CARGO_PKG_VERSION")));
        assert_eq!(VERSION.split('.').count(), 4);
    }

    #[test]
    fn the_fatal_text_names_the_reason_and_the_log() {
        let t = fatal_text("no game files: x", Path::new("C:/D2RUST/logs/crash.log"));
        assert!(t.starts_with("D2RUST stopped:"));
        assert!(t.contains("no game files: x"));
        assert!(t.ends_with("C:/D2RUST/logs/crash.log"));
    }

    #[test]
    fn log_directory_override_does_not_touch_documents() {
        const CHILD: &str = "D2RUST_LOG_DIR_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let dir = PathBuf::from(std::env::var_os(LOG_DIR_VAR).unwrap());
            assert_eq!(logs_dir(), Some(dir.clone()));
            assert_eq!(crash_log_path(), dir.join(CRASH_LOG));
            std::fs::write(dir.join(GAME_LOG), "previous run\n").unwrap();
            assert_eq!(open_log(), Some(dir.join(GAME_LOG)));
            log_line("isolated run");
            *LOG.lock().unwrap() = None;
            assert_eq!(
                std::fs::read_to_string(dir.join(GAME_LOG)).unwrap(),
                "isolated run\n"
            );
            assert_eq!(
                std::fs::read_to_string(dir.join(PREV_LOG)).unwrap(),
                "previous run\n"
            );
            assert!(!crate::user_dir::logs().exists());
            return;
        }

        // A separate process isolates both environment and the global log file.
        let temp = scratch("log-override");
        let dir = temp.join("output/logs");
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "launch::tests::log_directory_override_does_not_touch_documents",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env(LOG_DIR_VAR, &dir)
            .env("USERPROFILE", &temp)
            .env("HOME", &temp)
            .env_remove("D2RUST_HOME")
            .env_remove("D2RS_HOME")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(dir.join(GAME_LOG).is_file());
        assert!(!temp.join("Documents").exists());
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn the_game_log_keeps_the_previous_run() {
        let home = scratch("logs");
        let dir = home.join("logs");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(GAME_LOG), "run 1\n").unwrap();
        // What open_log does with the folder, without the global file.
        let path = dir.join(GAME_LOG);
        std::fs::rename(&path, dir.join(PREV_LOG)).unwrap();
        std::fs::write(&path, "run 2\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(PREV_LOG)).unwrap(),
            "run 1\n"
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "run 2\n");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn bevy_events_become_log_lines() {
        use bevy::log::tracing_subscriber::layer::SubscriberExt;
        let home = scratch("events");
        let path = home.join(GAME_LOG);
        *LOG.lock().unwrap() = Some(std::fs::File::create(&path).unwrap());
        let subscriber = bevy::log::tracing_subscriber::registry().with(FileLayer);
        bevy::log::tracing::subscriber::with_default(subscriber, || {
            bevy::log::tracing::warn!(target: "d2", count = 3, "slow frame");
        });
        *LOG.lock().unwrap() = None;
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "WARN d2: slow frame count=3\n");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn notes_keep_the_last() {
        for i in 0..NOTES + 3 {
            note(format!("n{i}"));
        }
        let n = notes();
        assert_eq!(n.len(), NOTES);
        assert_eq!(
            n.last().map(String::as_str),
            Some(&*format!("n{}", NOTES + 2))
        );
    }
}
