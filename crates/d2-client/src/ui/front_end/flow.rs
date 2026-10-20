//! The screen-flow table. Controls never name their target screen: they emit
//! a [`Trigger`] and [`next`] resolves it, so a screen module needs no edit
//! here. The conditions (saves found, difficulties open) come from [`FlowCtx`].

use super::ScreenId;

/// What happened on a screen (the flow's "Trigger" column).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Trademark: click on the image, a key of its list, or the 9 s timer.
    Continue,
    /// Main menu: Single Player.
    SinglePlayer,
    /// Main menu: Credits.
    Credits,
    /// Exit button or Esc (descriptors 5101 EXIT, hotkey 27).
    Exit,
    /// Character select: Create New.
    CreateNew,
    /// OK button or Enter (5102).
    Ok,
    /// Difficulty popup button: 0 Normal, 1 Nightmare, 2 Hell.
    Difficulty(u8),
    /// The in-game exit (`0x50` code 23 path).
    GameExit,
    /// Options menu: Configure Controls.
    ConfigureControls,
    /// Main menu: Options, the same pages as the in-game Options menu.
    Options,
    /// The current screen is torn down and built again (create's warning
    /// CANCEL).
    Rebuild,
    /// Main menu: Online (the online login screen).
    Online,
    /// An online screen lost its connection: back to the login screen,
    /// which shows why.
    OnlineLost,
}

/// How the game is started when a flow ends in a game load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameLoad {
    /// Chosen in the difficulty popup; `None`: the character's own (no box).
    pub difficulty: Option<u8>,
    /// Came from character create (a new character).
    pub new_character: bool,
    /// The online lobby joined a server game: the host takes the join
    /// (`screens::online::Online::take_joined`) and plays it over the
    /// network; the character is the server's, not a save.
    pub online: bool,
}

/// Where a trigger leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    Screen(ScreenId),
    /// Leave the front end: program exit.
    Exit,
    /// Leave the front end into the game.
    GameLoad(GameLoad),
    /// The trigger does nothing on this screen.
    Stay,
}

/// The facts the conditional rows read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlowCtx {
    /// `<save dir>*.d2s` found something.
    pub saves_found: bool,
    /// Difficulties open for the chosen character.
    pub difficulties_open: u8,
}

/// The front-end flow. `from` is the screen the trigger fired on.
pub fn next(from: ScreenId, t: Trigger, ctx: FlowCtx) -> Next {
    use super::screens::ids::*;
    use super::screens::online::{ONLINE_CHARS, ONLINE_CREATE, ONLINE_LOBBY, ONLINE_LOGIN};
    use Trigger::*;
    match (from, t) {
        // Online: login -> characters (-> create) -> lobby -> game; Back is
        // one step up, login's Back the main menu.
        (f, Online) if f == MAIN_MENU => Next::Screen(ONLINE_LOGIN),
        (f, Exit) if f == ONLINE_LOGIN => Next::Screen(MAIN_MENU),
        (f, Ok) if f == ONLINE_LOGIN => Next::Screen(ONLINE_CHARS),
        (f, Exit) if f == ONLINE_CHARS => Next::Screen(ONLINE_LOGIN),
        (f, CreateNew) if f == ONLINE_CHARS => Next::Screen(ONLINE_CREATE),
        (f, Ok) if f == ONLINE_CHARS => Next::Screen(ONLINE_LOBBY),
        (f, Exit | Ok) if f == ONLINE_CREATE => Next::Screen(ONLINE_CHARS),
        (f, Exit) if f == ONLINE_LOBBY => Next::Screen(ONLINE_CHARS),
        (f, Ok) if f == ONLINE_LOBBY => Next::GameLoad(GameLoad {
            difficulty: None,
            new_character: false,
            online: true,
        }),
        (_, OnlineLost) => Next::Screen(ONLINE_LOGIN),
        (f, Continue) if f == TRADEMARK => Next::Screen(MAIN_MENU),
        (f, SinglePlayer) if f == MAIN_MENU => {
            if ctx.saves_found {
                Next::Screen(CHAR_SELECT)
            } else {
                Next::Screen(CHAR_CREATE)
            }
        }
        (f, Exit) if f == MAIN_MENU => Next::Exit,
        (f, Credits) if f == MAIN_MENU => Next::Screen(CREDITS),
        // The credits leave to the main menu.
        (f, Exit) if f == CREDITS => Next::Screen(MAIN_MENU),
        (f, Exit) if f == CHAR_SELECT => Next::Screen(MAIN_MENU),
        (f, CreateNew) if f == CHAR_SELECT => Next::Screen(CHAR_CREATE),
        (f, Ok) if f == CHAR_SELECT => {
            if ctx.difficulties_open > 1 {
                Next::Screen(DIFFICULTY)
            } else {
                Next::GameLoad(GameLoad {
                    difficulty: None,
                    new_character: false,
                    online: false,
                })
            }
        }
        // Also when no character exists.
        (f, Exit) if f == CHAR_CREATE => Next::Screen(CHAR_SELECT),
        (f, Ok) if f == CHAR_CREATE => Next::GameLoad(GameLoad {
            difficulty: None,
            new_character: true,
            online: false,
        }),
        (f, Difficulty(d)) if f == DIFFICULTY => Next::GameLoad(GameLoad {
            difficulty: Some(d),
            new_character: false,
            online: false,
        }),
        // The box is a pop-up of the character select (`ui-frontend-charsel-ok`).
        (f, Difficulty(d)) if f == CHAR_SELECT => Next::GameLoad(GameLoad {
            difficulty: Some(d),
            new_character: false,
            online: false,
        }),
        (f, Exit) if f == DIFFICULTY => Next::Screen(CHAR_SELECT),
        // After Save and Exit the game shows the main menu: the host
        // opens `Entry::MainMenu`; `GameExit` (below) still opens the character
        // select. Configure Controls: Cancel / Accept return to the Options
        // menu (q-menu-controls).
        (f, Options) if f == MAIN_MENU => Next::Screen(OPTIONS),
        (f, Exit) if f == OPTIONS => Next::Screen(MAIN_MENU),
        (f, ConfigureControls) if f == OPTIONS => Next::Screen(CONTROLS),
        (f, Exit | Ok) if f == CONTROLS => Next::Screen(OPTIONS),
        (_, GameExit) => Next::Screen(CHAR_SELECT),
        _ => Next::Stay,
    }
}
