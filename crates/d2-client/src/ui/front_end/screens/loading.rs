//! The loading and act-change screen. `LoadingScreen` is a plain state
//! machine fed with the session events the client sees; it answers with
//! what is presented. Frame advance is per loading draw, never timed. The
//! screen ignores the pointer and does not filter keys:
//! bound keys go through the in-game key
//! path once S→C 0x01 has set key mode 1, so nothing here gates them.

use crate::ui::front_end::control::Control;
use crate::ui::front_end::screen::{FrontCtx, Screen};
use crate::ui::front_end::screens::ids::LOADING;
use crate::ui::front_end::Registry;

/// `loadingscreen.dc6`, under `data\global\ui\` (ENG, L3 r1).
pub const ART: &str = "Loading\\loadingscreen";
/// Language id other than 0 (L3 r1).
pub const ART_LOCAL: &str = r"DATA\LOCAL\UI\LoadingScreen";
/// Frames in the file (L3 r2).
pub const FRAMES: u32 = 10;
/// Palette set before the first draw (L3 r3).
pub const PALETTE: &str = r"DATA\GLOBAL\Palette\Loading\pal";

/// Art path for a language id (L3 r1).
pub fn art_path(language: u32) -> &'static str {
    if language == 0 {
        r"DATA\GLOBAL\UI\Loading\Loadingscreen"
    } else {
        ART_LOCAL
    }
}

/// Cel position (x, bottom y) of the art on a display (L4 r3).
pub fn placement(w: i32, h: i32) -> (i32, i32) {
    (w / 2 - 128, h / 2 + 128)
}

/// What one render presents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presented {
    /// Frame `frame` of the art on black, Loading palette.
    Loading { frame: u32 },
    /// The one black frame after the last loading draw (L7 r2).
    Black,
    /// The game frame, with the act palette `act<n+1>` (L7 r1).
    World { act: u8 },
}

/// A session event that matters here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadEvent {
    /// Game start: C→S 0x67 queued, then the init draw (L5 row 1).
    GameStart,
    /// S→C 0x01 (session start; key mode becomes 1).
    S01,
    /// S→C 0x03 (act load).
    S03 { act: u8 },
    /// S→C 0x04 (in game).
    S04,
    /// S→C 0x05 (act change: clears in_game).
    S05,
    /// S→C 0x61 (act-start video id).
    S61 { id: u8 },
}

/// Video ids that play over the loading screen (L10 r1 table).
pub fn video_name(id: u8) -> Option<&'static str> {
    Some(match id {
        2 => "ACT02START",
        3 => "ACT03START",
        4 => "ACT04START",
        5 => "ACT04END",
        _ => return None,
    })
}

#[derive(Debug, Default)]
pub struct LoadingScreen {
    /// Art held (a loading draw happened since the last game frame).
    art_held: bool,
    /// Frame counter.
    counter: u32,
    in_game: bool,
    act: u8,
    /// A loading draw is on screen / the screen is active.
    pub active: bool,
    /// Black frames still owed after the last loading draw.
    black_due: bool,
    /// A video played since the last loading draw: the screen shows black
    /// until the next loading draw, which the S→C 0x04 preload makes.
    pub after_video: bool,
    /// Video requests recorded by the stub (L10 r1).
    pub videos: Vec<u8>,
    /// The preload just drew: the game frame waits one client pass (L6
    /// r3).
    preload_held: bool,
    /// Everything presented, in order (consecutive repeats not merged).
    pub presented: Vec<Presented>,
}

impl LoadingScreen {
    pub fn new() -> Self {
        Self::default()
    }

    /// One loading draw (L4): clamp, draw, then counter += 1.
    fn draw(&mut self) {
        if !self.art_held {
            self.art_held = true;
            self.counter = 0;
        }
        if self.counter >= FRAMES {
            self.counter = FRAMES - 1;
        }
        self.presented.push(Presented::Loading {
            frame: self.counter,
        });
        self.counter += 1;
        self.active = true;
        self.black_due = true;
        self.after_video = false;
    }

    /// Feed one event.
    pub fn event(&mut self, e: LoadEvent) {
        match e {
            LoadEvent::GameStart => self.draw(),
            LoadEvent::S01 => {}
            LoadEvent::S03 { act } => {
                self.act = act;
                self.draw();
            }
            LoadEvent::S04 => self.in_game = true,
            LoadEvent::S05 => self.in_game = false,
            LoadEvent::S61 { id } => {
                if video_name(id).is_some() {
                    self.videos.push(id);
                    // The video itself does not
                    // redraw; black until the 0x04 preload draws again.
                    self.after_video = true;
                }
            }
        }
    }

    /// The room-graphics preload of S→C 0x04 (the room
    /// pass 0 with the preload flag set) over the `rooms` near
    /// rooms of the local player's room: a loading draw for every room
    /// but those where k & 0x1F = 0. Pass 1 draws nothing (its flag
    /// = 0 on a standard install). No room: no draw.
    pub fn preload(&mut self, rooms: usize) {
        for k in 0..rooms {
            if k & 0x1F != 0 {
                self.draw();
                self.preload_held = true;
            }
        }
    }

    /// A client loop pass with `placed` = the local player has a room
    /// (L6 r1, L7 r1–r2). Returns what is presented, if the game frame
    /// runs: a black frame first, then the world.
    pub fn frame(&mut self, placed: bool) -> Option<Presented> {
        if !(self.in_game && placed) {
            return None;
        }
        if std::mem::take(&mut self.preload_held) {
            // L6 r3 (D2RUST's own): the last preload picture stays presented
            // for this pass.
            return None;
        }
        if self.art_held {
            // Art freed, act palette loaded, one black frame (L7 r2).
            self.art_held = false;
            self.active = false;
            self.black_due = false;
            self.after_video = false;
            self.presented.push(Presented::Black);
            return Some(Presented::Black);
        }
        let w = Presented::World { act: self.act };
        self.presented.push(w);
        Some(w)
    }

    /// Pointer input is ignored while loading (L8 r1): never produces an intent.
    pub fn wants_input(&self) -> bool {
        !self.active
    }
}

struct Loading;

impl Screen for Loading {
    fn build(&mut self, _ctx: &mut FrontCtx) -> Vec<Control> {
        Vec::new()
    }
}

pub fn register(reg: &mut Registry) {
    reg.register(LOADING, Box::new(Loading));
}
