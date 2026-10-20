//! `autoplay-host --front`: the host starts on the front end's main menu
//! instead of in a game, so a route walks the menus the way a player does
//! (Single Player, a character, OK, a difficulty) before it plays.
//!
//! The front end runs as its model alone ([`FrontEnd`]): no window, no art,
//! one 40 ms tick per `step`, the host's pointer and key commands as its
//! input. Its screens are wired to the `--save-dir` folder as in `play`
//! ([`front_start::registry`]), so the character list shows that folder's
//! saves and the create screen checks its names. When the flow ends in a
//! game load, the choice becomes the game's character exactly as `play`
//! resolves it ([`StartChoice`], [`play_start::resolve`]) and the session
//! continues as a normal [`Host`].
//!
//! Clicks by id (`click ui:<id>`) and the `ui` list read
//! [`FrontEnd::ui_elements`]; every point is in the front end's 800 × 600
//! frame whatever `--res` says, since the front end always draws that
//! frame.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::autoplay_host::{Command, Host, HostArgs};
use super::front_host::DirSaves;
use super::front_start::{self, StartChoice, StartHandles};
use super::play_start::{self, CliStart};
use super::single_player::GameData;
use crate::ui::front_end::{FrontEnd, FrontInput, Outcome};
use crate::ui::ui_ids::{self, UiElement};
use crate::ui::PointerButton;

/// The front end of a `--front` session, until it starts the game.
pub struct FrontPhase {
    front: FrontEnd,
    handles: StartHandles,
    save_dir: PathBuf,
    data: GameData,
    game_dir: Option<PathBuf>,
    seed: Option<u32>,
    difficulty: u8,
    /// Front-end ticks run.
    pub passes: u64,
}

/// What a front-end command led to.
pub enum FrontStep {
    /// The reply; the front end is still up.
    Reply(Value),
    /// The flow started the game: the game's host, and the steps of the
    /// `step N` that are still to run in it.
    Game(Box<Host>, u32),
}

impl FrontPhase {
    /// The front end on the main menu, its screens wired to `--save-dir`.
    pub fn start(args: &HostArgs) -> Result<Self> {
        let game_dir = args
            .game_dir
            .clone()
            .or_else(|| std::env::var_os("D2_GAME_DIR").map(PathBuf::from));
        Self::start_with(args, GameData::select(game_dir.as_deref())?)
    }

    pub(crate) fn start_with(args: &HostArgs, data: GameData) -> Result<Self> {
        use d2_data::bin::TableFiles;
        let save_dir = args
            .save_dir
            .clone()
            .context("autoplay-host --front needs --save-dir DIR")?;
        if !save_dir.is_dir() {
            bail!("--save-dir {}: not a folder", save_dir.display());
        }
        let game_dir = args
            .game_dir
            .clone()
            .or_else(|| std::env::var_os("D2_GAME_DIR").map(PathBuf::from));
        let GameData::Live(live) = &data;
        let handles = StartHandles::default();
        let reg = front_start::registry(&save_dir, &handles);
        let mut front = FrontEnd::new(
            live.archives.lod(),
            Box::new(DirSaves(save_dir.clone())),
            reg,
        );
        // A later entry: straight to the main menu, no trademark screen.
        front.start(false);
        Ok(FrontPhase {
            front,
            handles,
            save_dir,
            data,
            game_dir,
            seed: args.seed,
            difficulty: args.difficulty,
            passes: 0,
        })
    }

    /// Stable control values, excluding animation clocks and pressed art.
    pub(crate) fn explorer_controls(&self) -> Value {
        // The settings too: an Options row whose value is an image changes
        // only them and pixels.
        json!({
            "controls": self
                .front
                .controls()
                .iter()
                .map(|c| (c.x, c.y, c.enabled, c.visible, c.checked, &c.text, c.cel))
                .collect::<Vec<_>>(),
            "settings": format!("{:?}", self.front.settings()),
        })
    }

    /// Draw requests read by the explorer's text oracle.
    pub(crate) fn explorer_draws(
        &mut self,
        adv: &dyn Fn(u16, &[u16]) -> i32,
    ) -> Vec<crate::ui::front_end::DrawItem> {
        let mut draws = self.front.draw();
        draws.extend(self.front.overlay(adv));
        draws
    }

    /// The front end's current screen.
    pub fn screen(&self) -> &'static str {
        self.front.current().0
    }

    /// Runs one command on the front end.
    pub fn run(&mut self, c: Command) -> Result<FrontStep> {
        let ok = |what: &str| Ok(FrontStep::Reply(json!({"k": "ok", "cmd": what})));
        match c {
            Command::Step(n) => {
                for done in 0..n {
                    self.front.tick();
                    self.passes += 1;
                    match self.front.outcome() {
                        Some(Outcome::GameLoad(_)) => {
                            let host = self.start_game()?;
                            return Ok(FrontStep::Game(Box::new(host), n - done - 1));
                        }
                        Some(Outcome::Exit) => bail!("the front end exited (Exit Game)"),
                        None => {}
                    }
                }
                Ok(FrontStep::Reply(json!({
                    "k": "ok", "cmd": "step", "passes": self.passes, "screen": self.screen(),
                })))
            }
            Command::Move(p) => {
                self.front.input(FrontInput::Move(p));
                ok("move")
            }
            // The front end takes the left button only.
            Command::Press(PointerButton::Left, p) => {
                self.front.input(FrontInput::Move(p));
                self.front.input(FrontInput::Down(p));
                ok("press")
            }
            Command::Release(PointerButton::Left, p) => {
                self.front.input(FrontInput::Move(p));
                self.front.input(FrontInput::Up(p));
                ok("release")
            }
            Command::Press(..) => ok("press"),
            Command::Release(..) => ok("release"),
            Command::Key(vk) | Command::RawKey(vk) => {
                let vk = u16::try_from(vk).context("key code")?;
                self.front.input(FrontInput::Key(vk));
                self.front.input(FrontInput::KeyUp(vk));
                ok("key")
            }
            Command::Text(text) => {
                for u in text.encode_utf16() {
                    self.front.input(FrontInput::Char(u));
                }
                ok("text")
            }
            Command::ClickUi(id) => {
                let els = self.front.ui_elements();
                let at = ui_ids::find(&els, &id).map_err(anyhow::Error::msg)?.at;
                self.front.input(FrontInput::Move(at));
                self.front.input(FrontInput::Down(at));
                self.front.input(FrontInput::Up(at));
                Ok(FrontStep::Reply(
                    json!({"k": "ok", "cmd": "click", "id": id, "at": [at.x, at.y]}),
                ))
            }
            Command::Ui => Ok(FrontStep::Reply(ui_reply(
                self.screen(),
                &self.front.ui_elements(),
            ))),
            Command::State => Ok(FrontStep::Reply(json!({
                "k": "state", "front": self.screen(), "passes": self.passes,
            }))),
            Command::Query(_)
            | Command::Map
            | Command::ClickUnit(_)
            | Command::InteractUnit(_)
            | Command::WalkLevel(_)
            | Command::UseUnit(_) => {
                bail!(
                    "the game has not started: the front end is on {}",
                    self.screen()
                )
            }
            Command::StepTicks(_) | Command::Pause | Command::Resume | Command::Shot(_) => {
                bail!("this command needs a running game")
            }
            Command::Quit => bail!("quit is handled by the caller"),
        }
    }

    /// The game the front end chose, started as `play` starts it.
    fn start_game(&mut self) -> Result<Host> {
        let Some(Outcome::GameLoad(load)) = self.front.outcome() else {
            bail!("the front end did not start a game");
        };
        let choice = StartChoice::resolve(load, &self.handles, &self.save_dir)
            .context("the front end chose no character")?;
        let cli = CliStart {
            save: None,
            new: None,
            save_dir: Some(self.save_dir.clone()),
            difficulty: self.difficulty,
            hardcore: false,
        };
        let start = play_start::resolve(
            &cli,
            &self.data,
            self.game_dir.as_deref(),
            load.difficulty,
            Some(&choice),
        )?;
        eprintln!("autoplay-host: {}", start.origin);
        Host::start_with(
            self.data.clone(),
            start.character,
            start.start_flags,
            start.hardcore,
            self.seed,
        )
    }
}

/// The `ui` reply: the screen and its elements (id, rectangle x y w h, the
/// point a click uses, slider ends, and why a click is idle on purpose).
pub fn ui_reply(screen: &str, els: &[UiElement]) -> Value {
    let ids: Vec<Value> = els
        .iter()
        .map(|e| {
            let mut value = json!({
                "id": e.id,
                "rect": [e.rect.x, e.rect.y, e.rect.w, e.rect.h],
                "at": [e.at.x, e.at.y],
            });
            if let Some(p) = e.ends {
                value["ends"] = json!([[p[0].x, p[0].y], [p[1].x, p[1].y]]);
            }
            if let Some(reason) = e.idle {
                value["idle"] = json!(reason);
            }
            value
        })
        .collect();
    json!({"k": "ui", "screen": screen, "ids": ids})
}
