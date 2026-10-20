//! `play --offscreen --control stdio` end to end on the user's game files:
//! a driver pauses the real game, steps whole ticks, queries it, takes a
//! shot of the presented picture and quits; and a run driven step by step
//! over the channel ends in the same server state (`state-1`) as the same
//! input given as an `--input` script.
//!
//! Real data (`D2_GAME_DIR`) and a GPU, so `#[ignore]`. The window is
//! hidden and never takes focus; nothing reads or moves the real mouse.
//! Shots show game art, so they go to the build's own temporary folder.
//! The parts that need no game (command parsing, message counts) are unit
//! tests in `app/control`.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::Value;

/// The walk target of the input: up and left of the player, inside every
/// play frame.
const AT: (i32, i32) = (300, 200);

/// A `play` game on the channel.
struct Game {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    /// The game's stderr, shown when it ends early.
    errors: PathBuf,
    /// The last log lines it printed on stdout, for the same.
    log: Vec<String>,
}

impl Game {
    /// `tag` names the run's own save folder, so runs never share one.
    fn start(tag: &str, extra: &[&str]) -> Self {
        let dir = std::env::var_os("D2_GAME_DIR").expect("D2_GAME_DIR");
        // The game writes the new character there but does not make the folder.
        std::fs::create_dir_all(tmp().join(tag)).expect("the save folder");
        let errors = tmp().join(format!("{tag}.stderr.log"));
        let mut child = Command::new(env!("CARGO_BIN_EXE_D2RUST-Game"))
            .args(["play", "--game-dir"])
            .arg(dir)
            .args(["--new", "sorceress", "Ctl", "--seed", "7", "--no-sound"])
            .arg("--save-dir")
            .arg(tmp().join(tag))
            .args(["--offscreen", "--size", "640x360", "--control", "stdio"])
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(&errors).expect("the stderr log"))
            .spawn()
            .expect("the game starts");
        let input = child.stdin.take().expect("stdin");
        let output = BufReader::new(child.stdout.take().expect("stdout"));
        Game {
            child,
            input,
            output,
            errors,
            log: Vec::new(),
        }
    }

    /// The next protocol line; the game's own log lines are skipped.
    fn reply(&mut self) -> Value {
        loop {
            let mut line = String::new();
            let n = self.output.read_line(&mut line).expect("read");
            if n == 0 {
                let errors = std::fs::read_to_string(&self.errors).unwrap_or_default();
                let lines: Vec<&str> = errors.lines().collect();
                let tail = lines[lines.len().saturating_sub(30)..].join("\n");
                panic!(
                    "the game ended without a reply\nstdout:\n{}\nstderr:\n{tail}",
                    self.log.join("\n"),
                );
            }
            if line.starts_with('{') {
                return serde_json::from_str(&line).expect("a JSON reply");
            }
            if self.log.len() == 30 {
                self.log.remove(0);
            }
            self.log.push(line.trim_end().to_string());
        }
    }

    fn send(&mut self, line: &str) -> Value {
        writeln!(self.input, "{line}").expect("write");
        self.input.flush().expect("flush");
        let r = self.reply();
        assert_ne!(r["k"], "error", "`{line}`: {r}");
        r
    }

    fn quit(mut self) {
        self.send("quit");
        drop(self.input);
        let status = self.child.wait().expect("the game exits");
        assert!(status.success(), "exit {status}");
    }
}

/// This test process's own folder under the build's temporary folder:
/// saves and shots of every run go there.
fn tmp() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("control-{}", std::process::id()))
}

fn tick(v: &Value) -> u64 {
    v["tick"].as_u64().expect("a tick")
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR and a GPU"]
fn a_driver_pauses_steps_queries_and_shoots() {
    let mut g = Game::start("shoot", &[]);
    let hello = g.reply();
    assert_eq!(hello["k"], "hello");
    assert_eq!(hello["format"], "autoplay-1");
    assert_eq!(hello["paused"], true);
    let start = tick(&hello);
    let frame = |s: &Value| s["snap"]["f"].as_i64().expect("server frame");
    let f0 = frame(&g.send("state"));

    // Held: passes run, ticks do not.
    let r = g.send("step 20");
    assert_eq!(tick(&r), start, "a held game does not tick");
    assert_eq!(frame(&g.send("state")), f0);

    let before = g.send("ui");
    assert_eq!(before["screen"], "game");
    g.send("click ui:hud.minipanel");
    g.send("step 2");
    assert_ne!(before["ids"], g.send("ui")["ids"]);
    g.send("click ui:hud.minipanel");
    g.send("step 2");
    assert_eq!(before["ids"], g.send("ui")["ids"]);

    let r = g.send("step-ticks 10");
    assert_eq!(tick(&r), start + 10);
    assert_eq!(frame(&g.send("state")), f0 + 10, "exactly 10 server frames");

    let q = g.send("query player.pos, hud.life");
    assert_eq!(q["k"], "query");
    assert!(q["values"]["player.pos"].is_array(), "{q}");

    let dir = tmp().join("shots");
    let path = dir.join("held.png");
    let s = g.send(&format!("shot {}", path.display()));
    assert_eq!(s["k"], "shot");
    assert_eq!(s["size"], serde_json::json!([640, 360]));
    assert!(path.is_file());

    // Resumed, the game ticks on its own clock; a pause holds it again.
    g.send("resume");
    let r = g.send("step 30");
    assert!(tick(&r) > start + 10, "a running game ticks");
    let held = tick(&g.send("pause"));
    let r = g.send("step 20");
    assert_eq!(tick(&r), held);
    g.quit();
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(tmp().join("shoot"));
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR and a GPU"]
fn a_stepped_run_matches_the_same_input_scripted() {
    // A walk: left button down after tick 59, up after tick 79, then 10
    // more ticks. `frame F` steps run after server tick F - 1, so the
    // script's `frame 60; hold X Y 20` is the press after tick 59 and the
    // release after tick 79.
    let (press, hold, end) = (59u64, 20u64, 89u64);
    let script = format!("frame {}; hold {} {} {hold}", press + 1, AT.0, AT.1);

    let mut a = Game::start("stepped", &[]);
    let start = tick(&a.reply());
    assert!(start < press, "the join ends at tick {start}");
    a.send(&format!("step-ticks {}", press - start));
    let before = a.send("state")["client"]["player"]["pos"].clone();
    assert!(!before.is_null(), "the player has a position");
    a.send(&format!("press L {} {}", AT.0, AT.1));
    a.send(&format!("step-ticks {hold}"));
    a.send(&format!("release L {} {}", AT.0, AT.1));
    let r = a.send(&format!("step-ticks {}", end - press - hold));
    assert_eq!(tick(&r), end);
    let stepped = a.send("state");
    a.quit();

    let mut b = Game::start("scripted", &["--input", &script]);
    let start_b = tick(&b.reply());
    assert_eq!(start_b, start, "both joins end on the same tick");
    let r = b.send(&format!("step-ticks {}", end - start));
    assert_eq!(tick(&r), end);
    let scripted = b.send("state");
    b.quit();
    for tag in ["stepped", "scripted"] {
        let _ = std::fs::remove_dir_all(tmp().join(tag));
    }

    assert!(stepped["snap"].is_object(), "{stepped}");
    assert_ne!(
        stepped["client"]["player"]["pos"], before,
        "the press walked the player"
    );
    assert_eq!(
        stepped["snap"], scripted["snap"],
        "the server state after tick {end} differs"
    );
}
