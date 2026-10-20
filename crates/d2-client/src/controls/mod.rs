//! The controls file (`d2controls 1`): which key or mouse input triggers which action.
//!
//! This module holds the strict parser, the writer, the built-in presets, the clash check and
//! the migration hook. It is plain Rust with no Bevy types; the mapping from Bevy's key codes to
//! [`Key`] lives in `d2-client::input`. Its submodules cover the rest of the controls system:
//! `original` is the full command table of the `original` preset, `keymap` translates between
//! its numeric codes and the portable names, `key_names` gives keys their on-screen names, and
//! `click` decides what a mouse click does in the world.
//!
//! Pipeline: TOML text becomes a [`RawFile`] (structure checked, names still strings, every
//! entry with its line), then [`migrate`] brings it to the current version, then it is resolved
//! into a [`ControlsFile`] (names resolved), then into [`Bindings`] (preset, overrides and
//! unbinds applied, clashes checked). Every error carries the line it was made on, and there is
//! no silent default: a bad file stops the client from starting.

pub mod click;
#[cfg(test)]
mod click_tests;
pub mod key_names;
pub mod keymap;
mod names;
pub mod original;
#[cfg(test)]
mod tests;

use std::fmt;
use std::ops::Range;
use std::path::Path;

pub use names::{action_by_name, Action, Context, Key};

/// Current format version (`d2controls 1`).
pub const VERSION: i64 = 1;

/// Inputs per action (two slots).
pub const MAX_INPUTS: usize = 2;

/// Base binding set a file starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// D2RUST's development preset. It is a convenience set of bindings for development, not the
    /// classic default configuration.
    Dev,
    /// The classic default key configuration: the 57 commands' 114 default bindings, plus the
    /// inputs that have no command ([`FIXED_INPUTS`]). This is the play default.
    Original,
}

impl Preset {
    /// Name as written in the controls file.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Dev => "dev",
            Preset::Original => "original",
        }
    }

    /// The preset's bindings, or `None` when the preset is not available (both current presets
    /// are).
    pub fn bindings(self) -> Option<Bindings> {
        match self {
            Preset::Dev => Some(dev_preset()),
            Preset::Original => Some(original_preset()),
        }
    }
}

/// The inputs of the actions that are not key-table commands: the fixed left / right mouse
/// buttons, and the chat and panel inputs (shared with the `dev` preset).
pub const FIXED_INPUTS: &[(Action, &[Key])] = {
    use Action as A;
    use Key as K;
    &[
        (A::MoveAttack, &[K::MouseLeft]),
        (A::UseRightSkill, &[K::MouseRight]),
        (A::ChatSend, &[K::Enter, K::NumpadEnter]),
        (A::ChatCancel, &[K::Escape]),
        (A::ChatHistoryPrev, &[K::Up]),
        (A::ChatHistoryNext, &[K::Down]),
        (A::PanelSelect, &[K::MouseLeft]),
        (A::PanelAlt, &[K::MouseRight]),
        (A::PanelClose, &[K::Escape]),
    ]
};

/// The `original` preset: every command of `original::COMMANDS` with its slot-1 then slot-0
/// default key, then [`FIXED_INPUTS`].
fn original_preset() -> Bindings {
    let mut b = Bindings::empty();
    for &(action, keys) in FIXED_INPUTS {
        b.set(action, keys);
    }
    for c in &original::COMMANDS {
        let Some(action) = keymap::action_of_cmd(i32::from(c.cmd)) else {
            continue;
        };
        let keys: Vec<Key> = [c.key1, c.key2]
            .into_iter()
            .filter(|&k| k != original::UNBOUND)
            .filter_map(keymap::vk_to_key)
            .collect();
        b.set(action, &keys);
    }
    b
}

/// The `dev` preset table (D2RUST's own development bindings, not the classic defaults).
const DEV_PRESET: &[(Action, &[Key])] = {
    use Action as A;
    use Key as K;
    &[
        (A::MoveAttack, &[K::MouseLeft]),
        (A::UseRightSkill, &[K::MouseRight]),
        (A::StandStill, &[K::LeftShift]),
        (A::ShowItems, &[K::LeftAlt]),
        (A::ToggleRun, &[K::R]),
        (A::SwapWeapons, &[K::W]),
        (A::ToggleInventory, &[K::I]),
        (A::ToggleCharacter, &[K::C]),
        (A::ToggleSkillTree, &[K::T]),
        (A::ToggleSkillMenuRight, &[K::S]),
        (A::ToggleQuests, &[K::Q]),
        (A::ToggleParty, &[K::P]),
        (A::ToggleAutomap, &[K::Tab]),
        (A::ToggleAutomapFade, &[K::F]),
        (A::ToggleBelt, &[K::Grave]),
        (A::ShowPortraits, &[K::Z]),
        (A::OpenChat, &[K::Enter]),
        (A::ClearScreen, &[K::Space]),
        (A::GameMenu, &[K::Escape]),
        (A::ToggleHelp, &[K::H]),
        (A::Screenshot, &[K::PrintScreen]),
        (A::SkillSlot1, &[K::F1]),
        (A::SkillSlot2, &[K::F2]),
        (A::SkillSlot3, &[K::F3]),
        (A::SkillSlot4, &[K::F4]),
        (A::SkillSlot5, &[K::F5]),
        (A::SkillSlot6, &[K::F6]),
        (A::SkillSlot7, &[K::F7]),
        (A::SkillSlot8, &[K::F8]),
        (A::SkillSlot9, &[K::F9]),
        (A::SkillSlot10, &[K::F10]),
        (A::SkillSlot11, &[K::F11]),
        (A::SkillSlot12, &[K::F12]),
        (A::BeltSlot1, &[K::Digit1]),
        (A::BeltSlot2, &[K::Digit2]),
        (A::BeltSlot3, &[K::Digit3]),
        (A::BeltSlot4, &[K::Digit4]),
        (A::ChatSend, &[K::Enter, K::NumpadEnter]),
        (A::ChatCancel, &[K::Escape]),
        (A::ChatHistoryPrev, &[K::Up]),
        (A::ChatHistoryNext, &[K::Down]),
        (A::PanelSelect, &[K::MouseLeft]),
        (A::PanelAlt, &[K::MouseRight]),
        (A::PanelClose, &[K::Escape]),
    ]
};

fn dev_preset() -> Bindings {
    let mut b = Bindings::empty();
    for &(action, keys) in DEV_PRESET {
        b.set(action, keys);
    }
    b
}

/// Effective bindings: up to [`MAX_INPUTS`] inputs per action, indexed by
/// [`Action::index`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings {
    slots: Vec<Vec<Key>>,
}

impl Bindings {
    /// No action bound.
    pub fn empty() -> Bindings {
        Bindings {
            slots: vec![Vec::new(); Action::ALL.len()],
        }
    }

    /// Inputs bound to `action`, in slot order.
    pub fn inputs(&self, action: Action) -> &[Key] {
        &self.slots[action.index()]
    }

    /// Replace `action`'s inputs (callers keep at most [`MAX_INPUTS`]).
    pub fn set(&mut self, action: Action, keys: &[Key]) {
        debug_assert!(keys.len() <= MAX_INPUTS);
        self.slots[action.index()] = keys.to_vec();
    }

    /// The action `key` triggers in `context`, if any.
    pub fn action_for(&self, context: Context, key: Key) -> Option<Action> {
        Action::ALL
            .iter()
            .copied()
            .find(|a| a.context() == context && self.inputs(*a).contains(&key))
    }

    /// First clash in `Action` enum order: two actions of one context
    /// sharing an input (one input may trigger at most one action per context). The pair is
    /// `(earlier, later)`.
    pub fn find_clash(&self) -> Option<Clash> {
        // Per context, the first action seen for each key.
        let mut seen: Vec<[Option<Action>; 3]> = vec![[None; 3]; Key::ALL.len()];
        for &action in Action::ALL {
            let ctx = action.context() as usize;
            for &key in self.inputs(action) {
                let slot = &mut seen[key as usize][ctx];
                match *slot {
                    Some(first) if first != action => {
                        return Some(Clash {
                            context: action.context(),
                            key,
                            first,
                            second: action,
                        });
                    }
                    _ => *slot = Some(action),
                }
            }
        }
        None
    }
}

/// Two actions of one context bound to the same input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clash {
    /// The context both actions are active in.
    pub context: Context,
    /// The shared input.
    pub key: Key,
    /// The earlier action in `Action` enum order.
    pub first: Action,
    /// The later action in `Action` enum order.
    pub second: Action,
}

/// A controls file with names resolved: preset, overrides, unbinds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlsFile {
    /// The base binding set the file starts from.
    pub preset: Preset,
    /// `[bindings]` entries, in file order (the writer uses enum order).
    pub bindings: Vec<(Action, Vec<Key>)>,
    /// `[unbind] list`, in file order.
    pub unbind: Vec<Action>,
}

impl ControlsFile {
    /// The smallest file over `preset` that yields `effective`: changed
    /// actions go to `[bindings]`, actions the preset binds and
    /// `effective` leaves empty go to `[unbind]`; both in enum order.
    pub fn from_effective(preset: Preset, effective: &Bindings) -> Option<ControlsFile> {
        let base = preset.bindings()?;
        let mut file = ControlsFile {
            preset,
            bindings: Vec::new(),
            unbind: Vec::new(),
        };
        for &action in Action::ALL {
            let want = effective.inputs(action);
            if want == base.inputs(action) {
                continue;
            }
            if want.is_empty() {
                file.unbind.push(action);
            } else {
                file.bindings.push((action, want.to_vec()));
            }
        }
        Some(file)
    }

    /// Effective bindings (the preset, then the file's overrides, then its unbinds) and the clash
    /// check. Lines in a clash error come from `lines`, when the file was parsed.
    fn resolve(&self, lines: Option<&Lines>) -> Result<Bindings, ControlsError> {
        let mut b = self.preset.bindings().ok_or(ControlsError {
            line: lines.map(|l| l.preset),
            kind: ErrorKind::PresetUnavailable(self.preset.name()),
        })?;
        for (action, keys) in &self.bindings {
            b.set(*action, keys);
        }
        for action in &self.unbind {
            b.set(*action, &[]);
        }
        if let Some(clash) = b.find_clash() {
            // Report at the file entry that caused it: the later action if
            // the file binds it, else the earlier one.
            let line = lines.and_then(|l| {
                l.binding_line(clash.second)
                    .or_else(|| l.binding_line(clash.first))
            });
            return Err(ControlsError {
                line,
                kind: ErrorKind::Clash(clash),
            });
        }
        Ok(b)
    }

    /// Effective bindings of a file built in code (no line numbers).
    pub fn effective(&self) -> Result<Bindings, ControlsError> {
        self.resolve(None)
    }
}

/// Lines of the parsed entries, for errors found after resolution.
struct Lines {
    preset: usize,
    bindings: Vec<(Action, usize)>,
}

impl Lines {
    fn binding_line(&self, action: Action) -> Option<usize> {
        self.bindings
            .iter()
            .find(|(a, _)| *a == action)
            .map(|(_, l)| *l)
    }
}

/// A file entry before name resolution: text and 1-based line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    /// The text as written in the file.
    pub name: String,
    /// 1-based line it was read from.
    pub line: usize,
}

/// A structurally valid file with names still unresolved; the unit
/// [`migrate`] works on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawFile {
    /// Format version the file declares.
    pub version: i64,
    /// The preset name, not yet resolved.
    pub preset: Named,
    /// `[bindings]`: action name, its line, input names with their lines.
    pub bindings: Vec<(Named, Vec<Named>)>,
    /// `[unbind] list`.
    pub unbind: Vec<Named>,
}

/// Migration hook: brings an older file to [`VERSION`] in memory. The file on disk is rewritten
/// only on an explicit user save.
/// Version 1 is the first format, so nothing is older yet; every version
/// outside `1..=VERSION` is an error.
pub fn migrate(raw: RawFile, version_line: usize) -> Result<RawFile, ControlsError> {
    match raw.version {
        VERSION => Ok(raw),
        v => Err(ControlsError::at(
            version_line,
            ErrorKind::UnsupportedVersion(v),
        )),
    }
}

/// A controls file error: what and on which line (1-based).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub struct ControlsError {
    /// `None` only for files built in code.
    pub line: Option<usize>,
    /// What went wrong.
    pub kind: ErrorKind,
}

impl ControlsError {
    fn at(line: usize, kind: ErrorKind) -> ControlsError {
        ControlsError {
            line: Some(line),
            kind,
        }
    }
}

impl fmt::Display for ControlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "controls file line {line}: {}", self.kind),
            None => write!(f, "controls: {}", self.kind),
        }
    }
}

/// What a controls file error is about.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ErrorKind {
    #[error("TOML syntax: {0}")]
    Toml(String),
    #[error("missing `version` (required, current is {VERSION})")]
    MissingVersion,
    #[error("unsupported version {0}")]
    UnsupportedVersion(i64),
    #[error("missing `preset`")]
    MissingPreset,
    #[error("unknown key `{0}`")]
    UnknownKey(String),
    #[error("`{key}` must be {expected}")]
    WrongType { key: String, expected: &'static str },
    #[error("unknown preset `{0}` (accepted: \"dev\")")]
    UnknownPreset(String),
    #[error("preset `{0}` is not available yet; use \"dev\"")]
    PresetUnavailable(&'static str),
    #[error("unknown action `{0}`")]
    UnknownAction(String),
    #[error("unknown input `{input}` for `{action}`")]
    UnknownInput { action: String, input: String },
    #[error("`{0}` has no inputs; list it in [unbind] instead")]
    EmptyBinding(String),
    #[error("`{0}` has more than {MAX_INPUTS} inputs")]
    TooManyInputs(String),
    #[error("`{action}` lists input `{input}` twice")]
    RepeatedInput { action: String, input: String },
    #[error("`{0}` listed twice in [unbind]")]
    RepeatedUnbind(String),
    #[error(
        "`{}` and `{}` both use `{}` in context `{}`",
        .0.first.name(), .0.second.name(), .0.key.name(), .0.context.name()
    )]
    Clash(Clash),
}

/// Parse a controls file into effective bindings (and the resolved file,
/// for a later save).
pub fn parse(text: &str) -> Result<(ControlsFile, Bindings), ControlsError> {
    let (raw, version_line) = parse_raw(text)?;
    let raw = migrate(raw, version_line)?;
    let (file, lines) = resolve_names(&raw)?;
    let bindings = file.resolve(Some(&lines))?;
    Ok((file, bindings))
}

/// Read and parse a controls file. The caller refuses to start on error.
pub fn load(path: &Path) -> anyhow::Result<(ControlsFile, Bindings)> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
    parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
}

/// Write a controls file: `version`, `preset`, then `[bindings]` and
/// `[unbind]` in `Action` enum order, whatever order `file`
/// holds them in.
pub fn write(file: &ControlsFile) -> String {
    let mut bindings: Vec<&(Action, Vec<Key>)> = file.bindings.iter().collect();
    bindings.sort_by_key(|(a, _)| *a);
    let mut unbind = file.unbind.clone();
    unbind.sort();
    unbind.dedup();

    let mut out = format!("version = {VERSION}\npreset = \"{}\"\n", file.preset.name());
    out.push_str("\n[bindings]\n");
    for (action, keys) in bindings {
        let list: Vec<String> = keys.iter().map(|k| format!("\"{}\"", k.name())).collect();
        out.push_str(&format!("{} = [{}]\n", action.name(), list.join(", ")));
    }
    out.push_str("\n[unbind]\n");
    let list: Vec<String> = unbind.iter().map(|a| format!("\"{}\"", a.name())).collect();
    out.push_str(&format!("list = [{}]\n", list.join(", ")));
    out
}

/// 1-based line of byte offset `at` in `text`.
fn line_of(text: &str, at: usize) -> usize {
    let at = at.min(text.len());
    text.as_bytes()[..at]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

/// Structural pass: TOML syntax, known keys, value types.
fn parse_raw(text: &str) -> Result<(RawFile, usize), ControlsError> {
    use toml_edit::{Document, Item, TableLike, Value};

    let doc = Document::parse(text).map_err(|e| {
        let line = e.span().map_or(1, |s| line_of(text, s.start));
        ControlsError::at(line, ErrorKind::Toml(e.message().to_string()))
    })?;
    let line = |span: Option<Range<usize>>| span.map_or(1, |s| line_of(text, s.start));
    let wrong = |key: &str, at: usize, expected: &'static str| {
        ControlsError::at(
            at,
            ErrorKind::WrongType {
                key: key.to_string(),
                expected,
            },
        )
    };
    // Line of a table entry: the value if it has a span, else the key.
    let entry_line = |t: &dyn TableLike, k: &str| -> usize {
        let (key, item) = t.get_key_value(k).expect("key from iteration");
        line(item.span().or_else(|| key.span()))
    };
    let key_line = |t: &dyn TableLike, k: &str| -> usize {
        let (key, item) = t.get_key_value(k).expect("key from iteration");
        line(key.span().or_else(|| item.span()))
    };
    // A string array, each element with its line.
    let strings = |key: &str, item: &Item, at: usize| -> Result<Vec<Named>, ControlsError> {
        let arr = item
            .as_array()
            .ok_or_else(|| wrong(key, at, "an array of strings"))?;
        arr.iter()
            .map(|v: &Value| {
                let vl = line(v.span()).max(at);
                v.as_str()
                    .map(|s| Named {
                        name: s.to_string(),
                        line: vl,
                    })
                    .ok_or_else(|| wrong(key, vl, "an array of strings"))
            })
            .collect()
    };

    let root = doc.as_table();
    let mut version: Option<(i64, usize)> = None;
    let mut preset: Option<Named> = None;
    let mut bindings = Vec::new();
    let mut unbind = Vec::new();

    // Version first, so a newer version is reported before any structure
    // that format may have changed. Older ones go to `migrate`.
    if let Some(item) = root.get("version") {
        let at = entry_line(root, "version");
        let v = item
            .as_integer()
            .ok_or_else(|| wrong("version", at, "an integer"))?;
        version = Some((v, at));
        if v > VERSION {
            return Err(ControlsError::at(at, ErrorKind::UnsupportedVersion(v)));
        }
    }
    let (version, version_line) =
        version.ok_or_else(|| ControlsError::at(1, ErrorKind::MissingVersion))?;

    for (k, item) in root.iter() {
        match k {
            "version" => {}
            "preset" => {
                let at = entry_line(root, k);
                let s = item.as_str().ok_or_else(|| wrong(k, at, "a string"))?;
                preset = Some(Named {
                    name: s.to_string(),
                    line: at,
                });
            }
            "bindings" => {
                let at = key_line(root, k);
                let t = item
                    .as_table_like()
                    .ok_or_else(|| wrong(k, at, "a table"))?;
                for (action, v) in t.iter() {
                    let al = key_line(t, action);
                    let inputs = strings(action, v, entry_line(t, action))?;
                    bindings.push((
                        Named {
                            name: action.to_string(),
                            line: al,
                        },
                        inputs,
                    ));
                }
            }
            "unbind" => {
                let at = key_line(root, k);
                let t = item
                    .as_table_like()
                    .ok_or_else(|| wrong(k, at, "a table"))?;
                for (uk, v) in t.iter() {
                    if uk != "list" {
                        return Err(ControlsError::at(
                            key_line(t, uk),
                            ErrorKind::UnknownKey(format!("unbind.{uk}")),
                        ));
                    }
                    unbind = strings("unbind.list", v, entry_line(t, uk))?;
                }
            }
            _ => {
                return Err(ControlsError::at(
                    key_line(root, k),
                    ErrorKind::UnknownKey(k.to_string()),
                ))
            }
        }
    }
    let preset = preset.ok_or_else(|| ControlsError::at(1, ErrorKind::MissingPreset))?;
    Ok((
        RawFile {
            version,
            preset,
            bindings,
            unbind,
        },
        version_line,
    ))
}

/// Name pass: preset, actions, inputs; per-entry rules.
fn resolve_names(raw: &RawFile) -> Result<(ControlsFile, Lines), ControlsError> {
    let preset = match raw.preset.name.as_str() {
        "dev" => Preset::Dev,
        "original" => Preset::Original,
        other => {
            return Err(ControlsError::at(
                raw.preset.line,
                ErrorKind::UnknownPreset(other.to_string()),
            ))
        }
    };
    let action = |n: &Named| {
        Action::from_name(&n.name)
            .ok_or_else(|| ControlsError::at(n.line, ErrorKind::UnknownAction(n.name.clone())))
    };

    let mut bindings = Vec::new();
    let mut lines = Lines {
        preset: raw.preset.line,
        bindings: Vec::new(),
    };
    // TOML itself rejects a key defined twice, so each action appears once.
    for (an, inputs) in &raw.bindings {
        let a = action(an)?;
        if inputs.is_empty() {
            return Err(ControlsError::at(
                an.line,
                ErrorKind::EmptyBinding(an.name.clone()),
            ));
        }
        if inputs.len() > MAX_INPUTS {
            return Err(ControlsError::at(
                inputs[MAX_INPUTS].line,
                ErrorKind::TooManyInputs(an.name.clone()),
            ));
        }
        let mut keys = Vec::new();
        for i in inputs {
            let k = Key::from_name(&i.name).ok_or_else(|| {
                ControlsError::at(
                    i.line,
                    ErrorKind::UnknownInput {
                        action: an.name.clone(),
                        input: i.name.clone(),
                    },
                )
            })?;
            if keys.contains(&k) {
                return Err(ControlsError::at(
                    i.line,
                    ErrorKind::RepeatedInput {
                        action: an.name.clone(),
                        input: i.name.clone(),
                    },
                ));
            }
            keys.push(k);
        }
        bindings.push((a, keys));
        lines.bindings.push((a, an.line));
    }

    let mut unbind = Vec::new();
    for n in &raw.unbind {
        let a = action(n)?;
        if unbind.contains(&a) {
            return Err(ControlsError::at(
                n.line,
                ErrorKind::RepeatedUnbind(n.name.clone()),
            ));
        }
        unbind.push(a);
    }

    Ok((
        ControlsFile {
            preset,
            bindings,
            unbind,
        },
        lines,
    ))
}
