//! Front-end music jukebox (`Options Music`):
//! out of game the music is a device-layer jukebox, not the sound table.
//! Plain Rust; the stream voice, the wall clock and the random source are
//! reached through [`JukeboxHost`] (D2RUST uses its own random
//! source and has no tick rule).

/// Entries of a playlist.
pub const PLAYLIST_LEN: usize = 8;

/// List A, paths under `data\global\music\`.
pub const LIST_A: [&str; PLAYLIST_LEN] = [
    "common\\options.wav",
    "act1\\caves.wav",
    "act1\\monastery.wav",
    "act1\\crypt.wav",
    "act2\\harem.wav",
    "act2\\tombs.wav",
    "act3\\spider.wav",
    "act3\\kurastsewer.wav",
];

/// List B, used when the expansion flag is set.
pub const LIST_B: [&str; PLAYLIST_LEN] = [
    "introedit.wav",
    "act5\\icecaves.wav",
    "act5\\xtemple.wav",
    "act2\\desert.wav",
    "act2\\sewer.wav",
    "act3\\kurast.wav",
    "act3\\kurastsewer.wav",
    "act4\\diablo.wav",
];

/// Voice volume of every front-end track; the player's Sound and Music
/// Volume scale it at the output (`FrontMusic::set_levels`).
pub const FRONT_END_VOLUME: i32 = 110;
/// Fade of a stopped, playing voice in wall-clock ms (toggle off).
pub const TOGGLE_FADE_MS: u32 = 200;
/// Device fade when leaving the front end (180).
pub const LEAVE_FADE_MS: u32 = 180;
/// Global gain G after leaving the front end.
pub const LEAVE_GAIN: i32 = 255;

/// The device layer under the jukebox.
pub trait JukeboxHost {
    /// CRT `rand()`.
    fn rand(&mut self) -> u32;
    /// The voice is playing.
    fn voice_playing(&self) -> bool;
    /// Reset the voice, set `volume`, start the stream `path` at offset 0
    /// without loop. False when
    /// the start failed.
    fn start_stream(&mut self, path: &str, volume: i32) -> bool;
    /// Fade a playing voice to volume 0 over `ms` of wall clock.
    fn fade_out(&mut self, ms: u32);
    /// Stop the voice.
    fn stop_voice(&mut self);
    /// Global gain G := `g`.
    fn set_global_gain(&mut self, g: i32);
}

/// Jukebox state: wanted, first-track, the played flags and the latched
/// list choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Jukebox {
    pub wanted: bool,
    pub first_track: bool,
    pub played: [bool; PLAYLIST_LEN],
    /// `Some(true)` = list B, latched on first use.
    latched_b: Option<bool>,
}

impl Jukebox {
    /// The playlist; `expansion` is the expansion flag, latched on
    /// first use.
    pub fn playlist(&mut self, expansion: bool) -> &'static [&'static str; PLAYLIST_LEN] {
        if *self.latched_b.get_or_insert(expansion) {
            &LIST_B
        } else {
            &LIST_A
        }
    }

    /// Want music (1): if not already wanted, clear every played
    /// flag and set first-track; then wanted := 1.
    pub fn start(&mut self) {
        if !self.wanted {
            self.played = [false; PLAYLIST_LEN];
            self.first_track = true;
        }
        self.wanted = true;
    }

    /// The track pick: called from the service pass while wanted and
    /// the voice is not playing. Returns the path started, if any.
    pub fn pick(&mut self, host: &mut dyn JukeboxHost, expansion: bool) -> Option<&'static str> {
        let list = self.playlist(expansion);
        let mut cleared = false;
        loop {
            if self.played.iter().all(|&p| p) {
                if cleared {
                    self.wanted = false;
                    return None;
                }
                self.played = [false; PLAYLIST_LEN];
                cleared = true;
            }
            let i = if self.first_track {
                self.first_track = false;
                0
            } else {
                let mut i = host.rand() as usize % PLAYLIST_LEN;
                while self.played[i] {
                    i = (i + 1) % PLAYLIST_LEN;
                }
                i
            };
            self.played[i] = true;
            if host.start_stream(list[i], FRONT_END_VOLUME) {
                return Some(list[i]);
            }
        }
    }

    /// The service gate: a pick is made only while wanted and the voice is idle.
    pub fn service(&mut self, host: &mut dyn JukeboxHost, expansion: bool) -> Option<&'static str> {
        if self.wanted && !host.voice_playing() {
            self.pick(host, expansion)
        } else {
            None
        }
    }

    /// The front-end options entry. Returns the new
    /// `Options Music`.
    pub fn toggle(&mut self, host: &mut dyn JukeboxHost, options_music: bool) -> bool {
        if options_music {
            // Wanted := 0; a playing voice fades out.
            self.wanted = false;
            if host.voice_playing() {
                host.fade_out(TOGGLE_FADE_MS);
            }
            false
        } else {
            self.start();
            true
        }
    }

    /// Every in-game client loop pass (and the Battle.net entry)
    /// while the voice plays: device fade, stop, G := 255.
    pub fn leave_front_end(&mut self, host: &mut dyn JukeboxHost) {
        if host.voice_playing() {
            host.fade_out(LEAVE_FADE_MS);
            self.wanted = false;
            host.stop_voice();
            host.set_global_gain(LEAVE_GAIN);
        }
    }
}
