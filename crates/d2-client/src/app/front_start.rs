//! The glue between the front end and the game start: what the select,
//! create and difficulty screens decided becomes the game's character, its
//! C→S 0x67 and its loading screen. No game logic: the
//! rules are the pure functions of `ui::front_end::screens::difficulty`.
//!
//! - [`StartHandles`] / [`registry`]: the shared cells the screens write
//!   their choice into, and a registry with them wired.
//! - [`StartChoice`]: the choice resolved from a [`GameLoad`]: the 0x67
//!   ([`StartChoice::create_request`]: name, class, difficulty,
//!   `start_flags`) and the character of the game.
//! - [`LoadFeed`] / [`LoadingState`]: the bridge's act / in-game state as
//!   [`LoadEvent`]s for the [`LoadingScreen`], and what to present.
//!
//! The game redraws the
//! loading screen on every S→C 0x03, the same act included (handler).
//! [`LoadFeed`] reads the model's session marks, one per 0x03 / 0x04 / 0x05 /
//! 0x61 in arrival order, so a repeated 0x03 draws again. `// D2RUST's own`:
//! the start act (`start_act`) is reported, not used to place the player (the
//! save load does that).

use std::path::{Path, PathBuf};

use crate::app::single_player::{self, Character};
use crate::bridge::world::ClientWorld;
use crate::ui::front_end::screens::char_select::{self, Selection, SelectionHandle};
use crate::ui::front_end::screens::create::{self, NewCharacter, NewCharacterSink};
use crate::ui::front_end::screens::difficulty::{self, start_flags};
use crate::ui::front_end::screens::loading::{LoadEvent, LoadingScreen, Presented};
use crate::ui::front_end::{GameLoad, Registry};
use d2_proto::client::CreateGame;

/// The cells the select and create screens leave their choice in.
#[derive(Clone, Default)]
pub struct StartHandles {
    pub selection: SelectionHandle,
    pub created: NewCharacterSink,
    /// Online play: the session the online screens open and the game joins
    /// through, and the screens' state ([`OnlineScreens`]: the join the
    /// lobby made, `take_joined`).
    #[cfg(feature = "online")]
    pub session: crate::app::online::OnlineSession,
    #[cfg(feature = "online")]
    pub online: OnlineScreens,
    /// `play --online --server ID`: the server the login screen starts on
    /// ([`Entry::Online`]); `None` keeps the list's first.
    #[cfg(feature = "online")]
    pub server: Option<String>,
}

impl StartHandles {
    /// The cells of a new front-end visit: the select and create cells
    /// start empty; online play's session and screens carry over.
    pub fn next_visit(&self) -> Self {
        Self {
            #[cfg(feature = "online")]
            session: self.session.clone(),
            #[cfg(feature = "online")]
            online: self.online.clone(),
            #[cfg(feature = "online")]
            server: self.server.clone(),
            ..Self::default()
        }
    }
}

/// The online screens' handle, once [`registry`] registered them.
#[cfg(feature = "online")]
pub type OnlineScreens =
    std::rc::Rc<std::cell::RefCell<Option<crate::ui::front_end::screens::online::OnlineHandle>>>;

/// A registry with every screen, select and create wired to `save_dir`
/// and `handles`.
pub fn registry(save_dir: &Path, handles: &StartHandles) -> Registry {
    let mut reg = Registry::default();
    crate::ui::front_end::screens::register_all(&mut reg);
    char_select::register_with(
        &mut reg,
        Some(save_dir.to_path_buf()),
        handles.selection.clone(),
    );
    let dir = save_dir.to_path_buf();
    create::register_with(
        &mut reg,
        handles.created.clone(),
        Box::new(move |name| create::name_taken_in(&dir, name)),
    );
    // The online screens on the real server (without the feature they
    // stay on `register_all`'s link, which says the build has none).
    // A front end opened again in the same process (after an online game)
    // keeps the screens' state: the lobby comes back on the same session.
    #[cfg(feature = "online")]
    {
        let kept = handles.online.borrow().clone();
        match kept {
            Some(online) => crate::ui::front_end::screens::online::register_with(&mut reg, online),
            None => {
                let online = crate::ui::front_end::screens::online::net_link::register(
                    &mut reg,
                    &handles.session,
                );
                *handles.online.borrow_mut() = Some(online);
            }
        }
    }
    reg
}

/// Where the front end opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// The first entry of the process: start-up chain and trademark (C1).
    First,
    /// A later entry, and back from a game (Save and Exit, or its window
    /// closed): the main menu, with no start-up chain or trademark.
    MainMenu,
    /// `play --online` (the launcher's "Play online"): the online login
    /// screen, with no start-up chain.
    Online,
    /// Back from an online game the player left (or one that did not
    /// start while the session lives): the lobby, on the same session.
    Lobby,
    /// Back from an online game whose connection is gone: the login screen,
    /// which shows why.
    OnlineError,
}

impl Entry {
    /// The screen the front end opens at, past its start (`None`: the
    /// start's own, the trademark or the main menu).
    fn screen(self) -> Option<crate::ui::front_end::ScreenId> {
        use crate::ui::front_end::screens::online::{ONLINE_LOBBY, ONLINE_LOGIN};
        match self {
            Entry::First | Entry::MainMenu => None,
            Entry::Online | Entry::OnlineError => Some(ONLINE_LOGIN),
            Entry::Lobby => Some(ONLINE_LOBBY),
        }
    }
}

/// Where the front end opens after an online game, and what the online
/// screens show there: `end` is why the game ended without the player
/// leaving it (`app::online::OnlineEnd`). With the session still open the
/// lobby shows the reason; with the session gone the login does.
#[cfg(feature = "online")]
pub fn after_online_game(handles: &StartHandles, end: Option<String>) -> Entry {
    let live = handles.session.0.lock().is_ok_and(|s| s.is_some());
    match handles.online.borrow().as_ref() {
        Some(online) => after_online_game_on(online, live, end),
        None => Entry::MainMenu,
    }
}

/// [`after_online_game`] on the screens' handle, `live` saying whether the
/// session is still open.
pub fn after_online_game_on(
    online: &crate::ui::front_end::screens::online::OnlineHandle,
    live: bool,
    end: Option<String>,
) -> Entry {
    let mut o = online.borrow_mut();
    // A join the game did not use is spent.
    o.take_joined();
    if live {
        o.state.notice = end;
        Entry::Lobby
    } else {
        o.lost(
            end.as_deref()
                .unwrap_or("The connection to the server was lost."),
        );
        Entry::OnlineError
    }
}

/// The front-end host `play` runs: every screen wired to `save_dir`
/// ([`registry`]), the credits text from `art`, the stub writer of a new
/// character; the handles the game start reads ([`StartChoice::resolve`]).
pub fn front_host(
    save_dir: &Path,
    art: Option<crate::app::front_host::FrontArt>,
    expansion: bool,
    entry: Entry,
) -> (crate::app::front_host::FrontHost, StartHandles) {
    let handles = StartHandles::default();
    let host = front_host_with(save_dir, art, expansion, entry, &handles);
    (host, handles)
}

/// [`front_host`] on `handles` the host keeps across games (online play:
/// the session and the online screens' state outlive each game).
pub fn front_host_with(
    save_dir: &Path,
    art: Option<crate::app::front_host::FrontArt>,
    expansion: bool,
    entry: Entry,
    handles: &StartHandles,
) -> crate::app::front_host::FrontHost {
    use crate::app::front_host::{register_credits, DirSaves, FrontHost};
    let mut reg = registry(save_dir, handles);
    if let Some(a) = &art {
        register_credits(&mut reg, a);
    }
    let front = crate::ui::front_end::FrontEnd::new(
        expansion,
        Box::new(DirSaves(save_dir.to_path_buf())),
        reg,
    );
    let mut host = FrontHost::with_front(front, art, entry == Entry::First)
        .with_stub_writer(handles.created.clone(), save_dir.to_path_buf());
    #[cfg(feature = "online")]
    if let (Entry::Online, Some(id), Some(online)) =
        (entry, &handles.server, handles.online.borrow().as_ref())
    {
        online.borrow_mut().select_server(id);
    }
    if let Some(screen) = entry.screen() {
        host.front.goto(screen);
    }
    host
}

/// The character the front end chose, and how to start it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartChoice {
    pub name: String,
    pub class: u8,
    /// The save's status word (new character: built from its choices).
    pub status: u16,
    /// Game difficulty 0–2.
    pub difficulty: u8,
    /// The save to load (`None`: a new character).
    pub save: Option<PathBuf>,
    /// The character's file in the Save folder, where the game writes it:
    /// the loaded save, or the stub `.d2s` the front end wrote for a new
    /// character (`FrontHost::with_stub_writer`).
    pub file: PathBuf,
}

impl StartChoice {
    /// Resolves the flow's end. `None`: the screens left no choice (the
    /// default character starts).
    pub fn resolve(load: GameLoad, handles: &StartHandles, save_dir: &Path) -> Option<StartChoice> {
        if load.new_character {
            let n: NewCharacter = handles.created.borrow_mut().take()?;
            let mut status = 0;
            if n.hardcore {
                status |= difficulty::STATUS_HARDCORE;
            }
            if n.expansion {
                status |= difficulty::STATUS_EXPANSION;
            }
            return Some(StartChoice {
                file: save_dir.join(format!("{}.d2s", n.name)),
                name: n.name,
                class: n.class.id(),
                status,
                difficulty: 0,
                save: None,
            });
        }
        let Selection { entry, .. } = handles.selection.lock().ok()?.take()?;
        let file = save_dir.join(format!("{}.d2s", entry.name));
        Some(StartChoice {
            save: Some(file.clone()),
            file,
            name: entry.name,
            class: entry.class,
            status: entry.status,
            // Without the box the game starts at Normal (L2 r1).
            difficulty: load.difficulty.unwrap_or(0),
        })
    }

    pub fn hardcore(&self) -> bool {
        self.status & difficulty::STATUS_HARDCORE != 0
    }

    /// The C→S 0x67 (L2 r5): the character's class and name, the game's
    /// difficulty and `start_flags` of its status.
    pub fn create_request(&self) -> CreateGame {
        let mut r = single_player::create_request_for(&self.named());
        r.flags = start_flags(self.status);
        r
    }

    /// The character by name and class alone (synthetic data, or a new
    /// character).
    fn named(&self) -> Character {
        let mut name = [0u8; 16];
        let n = self.name.len().min(15);
        name[..n].copy_from_slice(&self.name.as_bytes()[..n]);
        Character::Named(single_player::NewCharacter {
            class: self.class,
            name,
            difficulty: self.difficulty,
        })
    }

    /// The character the game loads: the save with live data, else the
    /// named stand-in.
    pub fn character(
        &self,
        data: &single_player::GameData,
    ) -> Result<Character, single_player::BuildError> {
        match (&self.save, data) {
            (Some(p), single_player::GameData::Live(_)) => {
                single_player::load_character(data, p, self.difficulty)
            }
            _ => Ok(self.named()),
        }
    }
}

/// Turns the model's session messages into [`LoadEvent`]s, in arrival
/// order (L9, L10): 0x05, 0x03, 0x61, 0x04, including a repeated 0x03 of
/// the same act. A model without session marks (state set directly) falls
/// back to edge detection.
#[derive(Debug, Default)]
pub struct LoadFeed {
    act: Option<crate::bridge::world::ActLoad>,
    in_game: bool,
    seen: u64,
}

impl LoadFeed {
    pub fn observe(&mut self, w: &ClientWorld) -> Vec<LoadEvent> {
        use crate::bridge::world::SessionMark;
        let mut out = Vec::new();
        let fresh = (w.session_total - self.seen) as usize;
        if fresh > 0 {
            let log = &w.session_log;
            // Marks older than the log's window are gone; take what is left.
            for m in &log[log.len() - fresh.min(log.len())..] {
                out.push(match *m {
                    SessionMark::LoadAct(act) => LoadEvent::S03 { act },
                    SessionMark::LoadComplete => LoadEvent::S04,
                    SessionMark::Unload => LoadEvent::S05,
                    SessionMark::Video(id) => LoadEvent::S61 { id },
                });
            }
            self.seen = w.session_total;
            self.act = w.act;
            self.in_game = w.in_game;
            return out;
        }
        if w.act != self.act {
            if let Some(a) = w.act {
                out.push(LoadEvent::S03 { act: a.act });
            }
            self.act = w.act;
        }
        if w.in_game != self.in_game {
            out.push(if w.in_game {
                LoadEvent::S04
            } else {
                LoadEvent::S05
            });
            self.in_game = w.in_game;
        }
        out
    }
}

/// The length of the local player's room list (the near rooms of the client
/// DRLG), 0 with no client DRLG or no placed player.
fn near_room_count(w: &ClientWorld) -> usize {
    match (w.drlg.as_ref(), w.local_room()) {
        (Some(cd), Some(r)) => cd.adjacency(r.room).len(),
        _ => 0,
    }
}

/// The loading screen of the running game.
#[derive(bevy::prelude::Resource)]
pub struct LoadingState {
    pub screen: LoadingScreen,
    feed: LoadFeed,
    /// What the last frame presented.
    pub last: Option<Presented>,
}

impl Default for LoadingState {
    /// Game start is the first loading draw (L5 row 1).
    fn default() -> Self {
        let mut screen = LoadingScreen::new();
        screen.event(LoadEvent::GameStart);
        Self {
            screen,
            feed: LoadFeed::default(),
            last: None,
        }
    }
}

impl LoadingState {
    /// One client loop pass over the model.
    pub fn frame(&mut self, w: &ClientWorld) -> Option<Presented> {
        let placed = w.local_room().is_some();
        self.advance(w, placed)
    }

    /// [`LoadingState::frame`] with `placed` (the local player has a room,
    /// L6 r1) given.
    pub fn advance(&mut self, w: &ClientWorld, placed: bool) -> Option<Presented> {
        for e in self.feed.observe(w) {
            self.screen.event(e);
            if e == LoadEvent::S04 {
                self.screen.preload(near_room_count(w));
            }
        }
        self.last = self.screen.frame(placed).or(self.last);
        match self.screen.presented.last().copied() {
            // Black, not the art, after an act video.
            Some(Presented::Loading { .. }) if self.screen.after_video => Some(Presented::Black),
            p => p,
        }
    }

    /// Whether the loading art covers the window.
    pub fn covering(&self) -> bool {
        self.screen.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::ActLoad;

    fn act(n: u8) -> ActLoad {
        ActLoad {
            act: n,
            init_seed: 1,
            town_level: 1,
            f8: 0,
        }
    }

    #[test]
    fn feed_emits_edges_once() {
        let mut f = LoadFeed::default();
        let mut w = ClientWorld::default();
        assert!(f.observe(&w).is_empty());
        w.act = Some(act(1));
        assert_eq!(f.observe(&w), vec![LoadEvent::S03 { act: 1 }]);
        assert!(f.observe(&w).is_empty());
        w.in_game = true;
        assert_eq!(f.observe(&w), vec![LoadEvent::S04]);
        w.in_game = false;
        assert_eq!(f.observe(&w), vec![LoadEvent::S05]);
    }

    #[test]
    fn loading_covers_until_the_world() {
        let mut s = LoadingState::default();
        assert!(s.covering());
        let mut w = ClientWorld {
            act: Some(act(0)),
            ..ClientWorld::default()
        };
        assert_eq!(s.frame(&w), Some(Presented::Loading { frame: 1 }));
        w.in_game = true;
        // In game but no player placed yet: still loading.
        assert!(matches!(s.frame(&w), Some(Presented::Loading { .. })));
        assert!(s.covering());
    }

    /// L5 row 4: game start presents 0 (init), 1 (0x03), then one preload
    /// draw per near room but the first (9 rooms: frames 2..9); the game
    /// frame waits one pass (L6 r3), then black, then the world.
    #[test]
    fn preload_draws_one_frame_per_room_but_every_32nd() {
        let mut s = LoadingScreen::new();
        s.event(LoadEvent::GameStart);
        s.event(LoadEvent::S03 { act: 0 });
        s.event(LoadEvent::S04);
        s.preload(9);
        let frames: Vec<u32> = s
            .presented
            .iter()
            .filter_map(|p| match p {
                Presented::Loading { frame } => Some(*frame),
                _ => None,
            })
            .collect();
        assert_eq!(frames, (0..10).collect::<Vec<_>>());
        assert_eq!(s.frame(true), None);
        assert_eq!(s.presented.last(), Some(&Presented::Loading { frame: 9 }));
        assert_eq!(s.frame(true), Some(Presented::Black));
        assert_eq!(s.frame(true), Some(Presented::World { act: 0 }));
        // k = 0 and k = 32 draw nothing; the counter clamps at 9.
        let mut s = LoadingScreen::new();
        s.preload(40);
        assert_eq!(s.presented.len(), 38);
        assert_eq!(s.presented.last(), Some(&Presented::Loading { frame: 9 }));
        // No room, no draw and no hold.
        let mut s = LoadingScreen::new();
        s.preload(0);
        assert!(s.presented.is_empty());
    }
}
