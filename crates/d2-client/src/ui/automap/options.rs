//! The automap options, their key commands and the cel files.
//! Open/close, size and re-centre live on [`super::Automap`] and
//! [`super::view::View`].

/// Registry value names (`HKCU\…\Diablo II`).
pub const FADE: &str = "AutoMapFade";
pub const CENTERS: &str = "AutoMap Centers";
pub const PARTY: &str = "AutoMap Party";
pub const PARTY_NAMES: &str = "AutoMap Party Names";
pub const LEFT: &str = "AutoMap Left";

/// The registry seam: DWORD values read at init and written by setters.
pub trait OptionStore {
    fn read(&self, name: &str) -> Option<u32>;
    fn write(&mut self, name: &str, value: u32);
}

/// An in-memory store (tests, or a session without settings).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryStore(pub std::collections::BTreeMap<String, u32>);

impl OptionStore for MemoryStore {
    fn read(&self, name: &str) -> Option<u32> {
        self.0.get(name).copied()
    }

    fn write(&mut self, name: &str, value: u32) {
        self.0.insert(name.to_owned(), value);
    }
}

/// The option globals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Options {
    /// Fade v.
    pub fade: u32,
    /// The fade latch: written by the fade setter only, so 0 after
    /// init.
    pub fade_latch: u32,
    /// Show the town centres.
    pub centers: bool,
    /// Show the party.
    pub party: bool,
    /// Show the party names.
    pub party_names: bool,
    /// Mini map on the left.
    pub left: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            fade: 0,
            fade_latch: 0,
            centers: true,
            party: true,
            party_names: true,
            left: true,
        }
    }
}

impl Options {
    /// Reads the five values, defaults when absent.
    pub fn init(store: &dyn OptionStore) -> Self {
        let d = Options::default();
        let flag = |n: &str, d: bool| store.read(n).map_or(d, |v| v != 0);
        Options {
            fade: store.read(FADE).unwrap_or(d.fade),
            fade_latch: 0,
            centers: flag(CENTERS, d.centers),
            party: flag(PARTY, d.party),
            party_names: flag(PARTY_NAMES, d.party_names),
            left: flag(LEFT, d.left),
        }
    }

    /// The fade setter: the registry value and both
    /// globals.
    pub fn set_fade(&mut self, v: u32, store: &mut dyn OptionStore) {
        self.fade = v;
        self.fade_latch = v;
        store.write(FADE, v);
    }

    /// F10 (cmd 9): value:= (value + 1) mod 4.
    pub fn cycle_fade(&mut self, store: &mut dyn OptionStore) {
        self.set_fade((self.fade + 1) % 4, store);
    }

    /// Options menu `AutoMap Centers`.
    pub fn set_centers(&mut self, on: bool, store: &mut dyn OptionStore) {
        self.centers = on;
        store.write(CENTERS, u32::from(on));
    }

    /// F11 (cmd 10), menu.
    pub fn set_party(&mut self, on: bool, store: &mut dyn OptionStore) {
        self.party = on;
        store.write(PARTY, u32::from(on));
    }

    /// F12 (cmd 11), menu.
    pub fn set_party_names(&mut self, on: bool, store: &mut dyn OptionStore) {
        self.party_names = on;
        store.write(PARTY_NAMES, u32::from(on));
    }

    /// V (cmd 45).
    pub fn set_left(&mut self, on: bool, store: &mut dyn OptionStore) {
        self.left = on;
        store.write(LEFT, u32::from(on));
    }

    pub fn toggle_party(&mut self, store: &mut dyn OptionStore) {
        self.set_party(!self.party, store);
    }

    pub fn toggle_party_names(&mut self, store: &mut dyn OptionStore) {
        self.set_party_names(!self.party_names, store);
    }

    pub fn toggle_left(&mut self, store: &mut dyn OptionStore) {
        self.set_left(!self.left, store);
    }
}

/// The automap cel file directory.
pub const CEL_DIR: &str = "DATA\\GLOBAL\\UI\\AutoMap\\";

/// The four cel file slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CelFile {
    MaxiMap = 0,
    Act2Map = 1,
    Act4Map = 2,
    ExTnMap = 3,
}

impl CelFile {
    pub const ALL: [CelFile; 4] = [
        CelFile::MaxiMap,
        CelFile::Act2Map,
        CelFile::Act4Map,
        CelFile::ExTnMap,
    ];

    fn base(self) -> &'static str {
        match self {
            CelFile::MaxiMap => "MaxiMap",
            CelFile::Act2Map => "Act2Map",
            CelFile::Act4Map => "Act4Map",
            CelFile::ExTnMap => "ExTnMap",
        }
    }

    /// The town file of a town kind; kind 0 draws with `MaxiMap`.
    pub fn of_town(kind: super::town::TownKind) -> CelFile {
        use super::town::TownKind as K;
        match kind {
            K::None => CelFile::MaxiMap,
            K::LutGholein => CelFile::Act2Map,
            K::Pandemonium => CelFile::Act4Map,
            K::Harrogath => CelFile::ExTnMap,
        }
    }
}

/// The path of each slot for the full (`mini` =
/// false) or mini size; `None` for a slot not loaded (`ExTnMap` of the
/// full size outside the expansion). The cel loader reads DC6 files: the
/// name gets `.dc6` (checked by the ignored `game_automap` test).
pub fn cel_paths(mini: bool, expansion: bool) -> [Option<String>; 4] {
    CelFile::ALL.map(|f| {
        let suffix = if mini { "S" } else { "" };
        if f == CelFile::ExTnMap && !mini && !expansion {
            return None;
        }
        Some(format!("{CEL_DIR}{}{suffix}.dc6", f.base()))
    })
}
