//! Case files: `render-cases/*.toml` (and `capture-cases/*.toml`), versioned with `version = 1`.
//! Parsing is strict input: unknown keys, wrong types, out-of-range numbers and a missing or
//! newer version are errors naming the key, never silently defaulted. A case references game
//! paths and parameters only, never game data, so case files can live in the repository.
//!
//! ```toml
//! version = 1
//! kind = "synthetic"          # or "map", "scene"
//! description = "..."         # optional, any kind
//!
//! # synthetic
//! view = [0, 0, 64, 64]       # x, y, width, height; default 800×600 frame
//! [[frame]]                   # FrameId(n) = n-th [[frame]]
//! width = 2
//! height = 2
//! pixels = [0, 5, 7, 0]       # or `fill = N`
//! [[map]]                     # one 256-entry row; shade = n-th [[map]]
//! base = "identity"           # or "zero"
//! set = [[5, 9]]              # (index, value), each index once
//! [[table]]                   # 256×256 blend table; table = n-th [[table]]
//! rule = "add"                # value of (src, dest): "src", "dest", "add", "xor"
//! [[item]]
//! frame = 0
//! x = 10
//! y = 10
//! clip = [0, 0, 800, 600]     # optional, default the frame
//! shade = [0]                 # optional, [[map]] indices, at most 4
//! table = 0                   # optional: IndexTable blend; absent = Opaque
//! key = [0, 0, 0, 0]          # optional: pass, major, minor, sub
//! flip_x = false              # optional (reserved; true is a scene error)
//! [[unit]]                    # a COF composite: COF bytes → items
//! cof = "03 02 02 14 ..."     # the COF file bytes in hex
//! dir = 0                     # COF direction
//! frame = 1                   # COF frame
//! key = [0, 5, 0]             # pass, major, minor; sub = slot
//! clip = [0, 0, 800, 600]     # optional, default the frame
//! [[unit.component]]          # the resolver's answer for one component
//! component = 0               # COF component id 0..=15, each at most once
//! frame = 0                   # [[frame]] index it draws
//! x = 10                      # screen top-left
//! y = 10
//! shade = [0]                 # optional, as for [[item]]
//! table = 0                   # optional, as for [[item]]
//! [[expect]]                  # index framebuffer value at a screen pixel
//! x = 11
//! y = 10
//! index = 5
//!
//! # map (a DS1 map composed from the game files)
//! ds1 = 'data\global\tiles\ACT1\TOWN\townN1.ds1'
//! wall_base = 80
//! view = [l, t, w, h]         # optional, default the whole map
//!
//! # scene (a captured reference recording; capture-cases/)
//! raw = "latest"              # or a frames-raw-1/-2 path (repo-relative or
//!                             # absolute); "latest" = newest
//!                             # traces/raw/*-frames.jsonl
//! images = 'game/captures/…'  # optional; default game/captures/<stamp>
//!                             # for a raw file named <stamp>-frames.jsonl
//! check = "compare"           # or "stability" (equal state, equal frame)
//! seqs = [1201, 1202]         # optional, compare only: these frames
//!                             # by the recorder's seq (1, 2, …;
//!                             # never the draw counter); default every
//!                             # captured frame but the first
//! ```

use std::fmt;

use toml_edit::{Document, Item, Table, Value};

/// The case file format version this build reads.
pub const VERSION: i64 = 1;

/// A parsed case. `name` is the file stem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub name: String,
    pub description: Option<String>,
    pub kind: CaseKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseKind {
    Synthetic(Synthetic),
    Map(MapCase),
    Scene(SceneCase),
}

impl CaseKind {
    pub fn name(&self) -> &'static str {
        match self {
            CaseKind::Synthetic(_) => "synthetic",
            CaseKind::Map(_) => "map",
            CaseKind::Scene(_) => "scene",
        }
    }
}

/// A view rectangle as written in a case: `[x, y, width, height]`.
pub type ViewRect = (i32, i32, u32, u32);

/// Items from inline frames and maps (repo only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synthetic {
    pub view: Option<ViewRect>,
    pub frames: Vec<FrameSpec>,
    pub maps: Vec<MapSpec>,
    pub tables: Vec<TableRule>,
    pub items: Vec<ItemSpec>,
    /// COF composites; their items follow `items` in the list before
    /// `scene::order` (equal keys keep that order).
    pub units: Vec<UnitSpec>,
    pub expects: Vec<Expect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameSpec {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapSpec {
    pub row: [u8; 256],
}

/// How a synthetic 256×256 blend table is filled: the value of each
/// (source, destination) pair. Tables are stored destination-major: row = destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableRule {
    Src,
    Dest,
    Add,
    Xor,
}

impl TableRule {
    pub fn value(self, src: u8, dest: u8) -> u8 {
        match self {
            TableRule::Src => src,
            TableRule::Dest => dest,
            TableRule::Add => src.wrapping_add(dest),
            TableRule::Xor => src ^ dest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSpec {
    pub frame: u32,
    pub x: i32,
    pub y: i32,
    pub clip: Option<ViewRect>,
    pub shade: Vec<u32>,
    pub table: Option<u32>,
    pub key: Option<[u32; 4]>,
    pub flip_x: bool,
}

/// One COF composite (`[[unit]]`). The COF is given as file bytes so the
/// case runs the real parser (`d2_formats::cof`); everything the game rules
/// would decide (frame, position, shade, blend) is the case's fixture answer
/// per component, so the case tests the composite path, not those rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitSpec {
    pub cof: Vec<u8>,
    pub dir: u32,
    pub frame: u32,
    pub key: [u32; 3],
    pub clip: Option<ViewRect>,
    pub components: Vec<ComponentSpec>,
}

/// The fixture answer for one COF component of a unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentSpec {
    pub component: u8,
    pub frame: u32,
    pub x: i32,
    pub y: i32,
    pub shade: Vec<u32>,
    pub table: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expect {
    pub x: i32,
    pub y: i32,
    pub index: u8,
}

/// A DS1 map composed from the game files (the map preview's verify).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapCase {
    pub ds1: String,
    pub wall_base: i32,
    pub view: Option<ViewRect>,
}

/// A captured reference recording compared with the D2RUST render pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneCase {
    pub raw: RawRef,
    pub images: Option<String>,
    pub check: CaptureCheck,
    /// Frames to compare, by the recorder's `seq` (never the draw counter);
    /// empty = the default selection.
    pub seqs: Vec<u32>,
}

/// Which raw capture file a scene case reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawRef {
    /// The newest `traces/raw/*-frames.jsonl` (file names start with the
    /// recording time, so the newest sorts last).
    Latest,
    Path(String),
}

/// What a scene case checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureCheck {
    /// Each selected frame against the CPU reference (and GPU) of its
    /// recorded state.
    Compare,
    /// The stability rule: frames with equal recorded state have equal
    /// pixels. Needs no renderer.
    Stability,
}

/// A case file error: the key path (`item[2].frame`) and what is wrong.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct CaseError {
    pub at: String,
    pub kind: CaseErrorKind,
}

impl fmt::Display for CaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.at, self.kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaseErrorKind {
    #[error("TOML syntax: {0}")]
    Toml(String),
    #[error("missing `version` (this build reads version {VERSION})")]
    MissingVersion,
    #[error("version {0} is not supported (this build reads version {VERSION})")]
    UnsupportedVersion(i64),
    #[error("missing required key")]
    Missing,
    #[error("unknown key")]
    UnknownKey,
    #[error("expected {0}")]
    WrongType(&'static str),
    #[error("{value} is out of range {min}..={max}")]
    Range { value: i64, min: i64, max: i64 },
    #[error("unknown value {0:?}")]
    UnknownValue(String),
    #[error("{0}")]
    Invalid(String),
}

fn err(at: impl Into<String>, kind: CaseErrorKind) -> CaseError {
    CaseError {
        at: at.into(),
        kind,
    }
}

/// Parses one case file; `name` is its file stem.
pub fn parse(name: &str, text: &str) -> Result<Case, CaseError> {
    let doc = Document::parse(text).map_err(|e| {
        let line = e.span().map_or(1, |s| line_of(text, s.start));
        err(
            format!("line {line}"),
            CaseErrorKind::Toml(e.message().to_string()),
        )
    })?;
    let root = doc.as_table();
    // Version first, so a newer format is reported before anything it may
    // have changed.
    let version = root
        .get("version")
        .ok_or_else(|| err("version", CaseErrorKind::MissingVersion))?;
    let version = version
        .as_integer()
        .ok_or_else(|| err("version", CaseErrorKind::WrongType("an integer")))?;
    if version != VERSION {
        return Err(err("version", CaseErrorKind::UnsupportedVersion(version)));
    }
    let kind = string(root, "", "kind")?.ok_or_else(|| err("kind", CaseErrorKind::Missing))?;
    let description = string(root, "", "description")?;
    let common = ["version", "kind", "description"];
    let kind = match kind.as_str() {
        "synthetic" => {
            only_keys(
                root,
                "",
                &[
                    &common[..],
                    &["view", "frame", "map", "table", "item", "unit", "expect"],
                ]
                .concat(),
            )?;
            CaseKind::Synthetic(synthetic(root)?)
        }
        "map" => {
            only_keys(
                root,
                "",
                &[&common[..], &["ds1", "wall_base", "view"]].concat(),
            )?;
            CaseKind::Map(MapCase {
                ds1: string(root, "", "ds1")?.ok_or_else(|| err("ds1", CaseErrorKind::Missing))?,
                wall_base: int(root, "", "wall_base", i32::MIN.into(), i32::MAX.into())?
                    .ok_or_else(|| err("wall_base", CaseErrorKind::Missing))?
                    as i32,
                view: rect(root, "", "view")?,
            })
        }
        "scene" => {
            only_keys(
                root,
                "",
                &[&common[..], &["raw", "images", "check", "seqs"]].concat(),
            )?;
            CaseKind::Scene(scene(root)?)
        }
        other => return Err(err("kind", CaseErrorKind::UnknownValue(other.to_owned()))),
    };
    Ok(Case {
        name: name.to_owned(),
        description,
        kind,
    })
}

fn synthetic(root: &Table) -> Result<Synthetic, CaseError> {
    let frames = tables(root, "frame")?
        .into_iter()
        .map(|(at, t)| frame(&at, t))
        .collect::<Result<_, _>>()?;
    let maps = tables(root, "map")?
        .into_iter()
        .map(|(at, t)| map(&at, t))
        .collect::<Result<_, _>>()?;
    let tables_ = tables(root, "table")?
        .into_iter()
        .map(|(at, t)| {
            only_keys(t, &at, &["rule"])?;
            let rule = string(t, &at, "rule")?
                .ok_or_else(|| err(path(&at, "rule"), CaseErrorKind::Missing))?;
            Ok(match rule.as_str() {
                "src" => TableRule::Src,
                "dest" => TableRule::Dest,
                "add" => TableRule::Add,
                "xor" => TableRule::Xor,
                _ => return Err(err(path(&at, "rule"), CaseErrorKind::UnknownValue(rule))),
            })
        })
        .collect::<Result<_, _>>()?;
    let items: Vec<ItemSpec> = tables(root, "item")?
        .into_iter()
        .map(|(at, t)| item(&at, t))
        .collect::<Result<_, _>>()?;
    let units: Vec<UnitSpec> = tables(root, "unit")?
        .into_iter()
        .map(|(at, t)| unit(&at, t))
        .collect::<Result<_, _>>()?;
    if items.is_empty() && units.is_empty() {
        return Err(err(
            "item",
            CaseErrorKind::Invalid(
                "a synthetic case needs at least one [[item]] or [[unit]]".into(),
            ),
        ));
    }
    let expects = tables(root, "expect")?
        .into_iter()
        .map(|(at, t)| {
            only_keys(t, &at, &["x", "y", "index"])?;
            Ok(Expect {
                x: req_int(t, &at, "x", i32::MIN.into(), i32::MAX.into())? as i32,
                y: req_int(t, &at, "y", i32::MIN.into(), i32::MAX.into())? as i32,
                index: req_int(t, &at, "index", 0, 255)? as u8,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(Synthetic {
        view: rect(root, "", "view")?,
        frames,
        maps,
        tables: tables_,
        items,
        units,
        expects,
    })
}

fn scene(root: &Table) -> Result<SceneCase, CaseError> {
    let raw = string(root, "", "raw")?.ok_or_else(|| err("raw", CaseErrorKind::Missing))?;
    let raw = match raw.as_str() {
        "latest" => RawRef::Latest,
        "" => return Err(err("raw", CaseErrorKind::Invalid("empty path".into()))),
        _ => RawRef::Path(raw),
    };
    let check = string(root, "", "check")?.ok_or_else(|| err("check", CaseErrorKind::Missing))?;
    let check = match check.as_str() {
        "compare" => CaptureCheck::Compare,
        "stability" => CaptureCheck::Stability,
        _ => return Err(err("check", CaseErrorKind::UnknownValue(check))),
    };
    let seqs = match root.get("seqs") {
        None => Vec::new(),
        Some(item) => {
            let arr = item
                .as_array()
                .ok_or_else(|| err("seqs", CaseErrorKind::WrongType("an array of integers")))?;
            let mut seqs = Vec::with_capacity(arr.len());
            for (i, v) in arr.iter().enumerate() {
                let at = format!("seqs[{i}]");
                let d = value_int(v, &at, 0, u32::MAX.into())? as u32;
                if seqs.contains(&d) {
                    return Err(err(
                        at,
                        CaseErrorKind::Invalid(format!("frame {d} is listed twice")),
                    ));
                }
                seqs.push(d);
            }
            if seqs.is_empty() {
                return Err(err(
                    "seqs",
                    CaseErrorKind::Invalid("empty; omit it for the default".into()),
                ));
            }
            if check == CaptureCheck::Stability {
                return Err(err(
                    "seqs",
                    CaseErrorKind::Invalid("the stability check uses every frame".into()),
                ));
            }
            seqs
        }
    };
    Ok(SceneCase {
        raw,
        images: string(root, "", "images")?,
        check,
        seqs,
    })
}

fn frame(at: &str, t: &Table) -> Result<FrameSpec, CaseError> {
    only_keys(t, at, &["width", "height", "pixels", "fill"])?;
    let width = req_int(t, at, "width", 1, 4096)? as u32;
    let height = req_int(t, at, "height", 1, 4096)? as u32;
    let len = width as usize * height as usize;
    let pixels = match (t.get("pixels"), int(t, at, "fill", 0, 255)?) {
        (Some(_), Some(_)) => {
            return Err(err(
                at,
                CaseErrorKind::Invalid("give `pixels` or `fill`, not both".into()),
            ))
        }
        (None, Some(fill)) => vec![fill as u8; len],
        (Some(item), None) => {
            let key = path(at, "pixels");
            let arr = item
                .as_array()
                .ok_or_else(|| err(&key, CaseErrorKind::WrongType("an array of integers")))?;
            let pixels: Vec<u8> = arr
                .iter()
                .enumerate()
                .map(|(i, v)| value_int(v, &format!("{key}[{i}]"), 0, 255).map(|v| v as u8))
                .collect::<Result<_, _>>()?;
            if pixels.len() != len {
                return Err(err(
                    key,
                    CaseErrorKind::Invalid(format!(
                        "{} pixels for a {width}x{height} frame",
                        pixels.len()
                    )),
                ));
            }
            pixels
        }
        (None, None) => return Err(err(path(at, "pixels"), CaseErrorKind::Missing)),
    };
    Ok(FrameSpec {
        width,
        height,
        pixels,
    })
}

fn map(at: &str, t: &Table) -> Result<MapSpec, CaseError> {
    only_keys(t, at, &["base", "set"])?;
    let base =
        string(t, at, "base")?.ok_or_else(|| err(path(at, "base"), CaseErrorKind::Missing))?;
    let mut row = [0u8; 256];
    match base.as_str() {
        "identity" => row.iter_mut().enumerate().for_each(|(i, v)| *v = i as u8),
        "zero" => {}
        _ => return Err(err(path(at, "base"), CaseErrorKind::UnknownValue(base))),
    }
    let mut seen = [false; 256];
    if let Some(item) = t.get("set") {
        let key = path(at, "set");
        let pairs = item
            .as_array()
            .ok_or_else(|| err(&key, CaseErrorKind::WrongType("an array of [index, value]")))?;
        for (i, pair) in pairs.iter().enumerate() {
            let pk = format!("{key}[{i}]");
            let [index, value] = int_array::<2>(pair, &pk, 0, 255)?;
            if std::mem::replace(&mut seen[index as usize], true) {
                return Err(err(
                    pk,
                    CaseErrorKind::Invalid(format!("index {index} is set twice")),
                ));
            }
            row[index as usize] = value as u8;
        }
    }
    Ok(MapSpec { row })
}

fn item(at: &str, t: &Table) -> Result<ItemSpec, CaseError> {
    only_keys(
        t,
        at,
        &["frame", "x", "y", "clip", "shade", "table", "key", "flip_x"],
    )?;
    let shade = shade(t, at)?;
    let key = match t.get("key") {
        None => None,
        Some(item) => {
            let v = item
                .as_value()
                .ok_or_else(|| err(path(at, "key"), CaseErrorKind::WrongType("an array")))?;
            Some(int_array::<4>(v, &path(at, "key"), 0, u32::MAX.into())?.map(|v| v as u32))
        }
    };
    let flip_x = match t.get("flip_x") {
        None => false,
        Some(item) => item
            .as_bool()
            .ok_or_else(|| err(path(at, "flip_x"), CaseErrorKind::WrongType("a boolean")))?,
    };
    Ok(ItemSpec {
        frame: req_int(t, at, "frame", 0, u32::MAX.into())? as u32,
        x: req_int(t, at, "x", i32::MIN.into(), i32::MAX.into())? as i32,
        y: req_int(t, at, "y", i32::MIN.into(), i32::MAX.into())? as i32,
        clip: rect(t, at, "clip")?,
        shade,
        table: int(t, at, "table", 0, u32::MAX.into())?.map(|v| v as u32),
        key,
        flip_x,
    })
}

fn unit(at: &str, t: &Table) -> Result<UnitSpec, CaseError> {
    only_keys(t, at, &["cof", "dir", "frame", "key", "clip", "component"])?;
    let cof = hex(t, at, "cof")?.ok_or_else(|| err(path(at, "cof"), CaseErrorKind::Missing))?;
    let key_at = path(at, "key");
    let key = t
        .get("key")
        .ok_or_else(|| err(&key_at, CaseErrorKind::Missing))?
        .as_value()
        .ok_or_else(|| err(&key_at, CaseErrorKind::WrongType("an array")))?;
    let key = int_array::<3>(key, &key_at, 0, u32::MAX.into())?.map(|v| v as u32);
    let mut components: Vec<ComponentSpec> = Vec::new();
    for (cat, c) in tables(t, "component")? {
        let cat = path(at, &cat);
        only_keys(c, &cat, &["component", "frame", "x", "y", "shade", "table"])?;
        let component = req_int(c, &cat, "component", 0, 15)? as u8;
        if components.iter().any(|o| o.component == component) {
            return Err(err(
                path(&cat, "component"),
                CaseErrorKind::Invalid(format!("component {component} is given twice")),
            ));
        }
        components.push(ComponentSpec {
            component,
            frame: req_int(c, &cat, "frame", 0, u32::MAX.into())? as u32,
            x: req_int(c, &cat, "x", i32::MIN.into(), i32::MAX.into())? as i32,
            y: req_int(c, &cat, "y", i32::MIN.into(), i32::MAX.into())? as i32,
            shade: shade(c, &cat)?,
            table: int(c, &cat, "table", 0, u32::MAX.into())?.map(|v| v as u32),
        });
    }
    Ok(UnitSpec {
        cof,
        dir: req_int(t, at, "dir", 0, 255)? as u32,
        frame: req_int(t, at, "frame", 0, 255)? as u32,
        key,
        clip: rect(t, at, "clip")?,
        components,
    })
}

// --- strict readers ---------------------------------------------------------

/// `shade = [..]`: map indices, empty when absent.
fn shade(t: &Table, at: &str) -> Result<Vec<u32>, CaseError> {
    let Some(item) = t.get("shade") else {
        return Ok(Vec::new());
    };
    let key = path(at, "shade");
    let arr = item
        .as_array()
        .ok_or_else(|| err(&key, CaseErrorKind::WrongType("an array of integers")))?;
    arr.iter()
        .enumerate()
        .map(|(i, v)| value_int(v, &format!("{key}[{i}]"), 0, u32::MAX.into()))
        .map(|r| r.map(|v| v as u32))
        .collect()
}

/// A byte string in hex, two digits per byte; ASCII whitespace between
/// bytes is ignored. Anything else is an error.
fn hex(t: &Table, at: &str, key: &str) -> Result<Option<Vec<u8>>, CaseError> {
    let Some(text) = string(t, at, key)? else {
        return Ok(None);
    };
    let k = path(at, key);
    let mut out = Vec::new();
    for word in text.split_ascii_whitespace() {
        if word.len() % 2 != 0 {
            return Err(err(
                &k,
                CaseErrorKind::Invalid(format!("{word:?} is not whole hex bytes")),
            ));
        }
        for pair in word.as_bytes().chunks(2) {
            let digits = std::str::from_utf8(pair).unwrap_or("");
            let byte = u8::from_str_radix(digits, 16)
                .ok()
                .filter(|_| pair.iter().all(u8::is_ascii_hexdigit))
                .ok_or_else(|| err(&k, CaseErrorKind::Invalid(format!("{word:?} is not hex"))))?;
            out.push(byte);
        }
    }
    Ok(Some(out))
}

fn path(at: &str, key: &str) -> String {
    if at.is_empty() {
        key.to_owned()
    } else {
        format!("{at}.{key}")
    }
}

fn only_keys(t: &Table, at: &str, allowed: &[&str]) -> Result<(), CaseError> {
    match t.iter().find(|(k, _)| !allowed.contains(k)) {
        Some((k, _)) => Err(err(path(at, k), CaseErrorKind::UnknownKey)),
        None => Ok(()),
    }
}

fn string(t: &Table, at: &str, key: &str) -> Result<Option<String>, CaseError> {
    t.get(key)
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| err(path(at, key), CaseErrorKind::WrongType("a string")))
        })
        .transpose()
}

fn value_int(v: &Value, at: &str, min: i64, max: i64) -> Result<i64, CaseError> {
    let value = v
        .as_integer()
        .ok_or_else(|| err(at, CaseErrorKind::WrongType("an integer")))?;
    if !(min..=max).contains(&value) {
        return Err(err(at, CaseErrorKind::Range { value, min, max }));
    }
    Ok(value)
}

fn int(t: &Table, at: &str, key: &str, min: i64, max: i64) -> Result<Option<i64>, CaseError> {
    t.get(key)
        .map(|item| {
            let k = path(at, key);
            let v = item
                .as_value()
                .ok_or_else(|| err(&k, CaseErrorKind::WrongType("an integer")))?;
            value_int(v, &k, min, max)
        })
        .transpose()
}

fn req_int(t: &Table, at: &str, key: &str, min: i64, max: i64) -> Result<i64, CaseError> {
    int(t, at, key, min, max)?.ok_or_else(|| err(path(at, key), CaseErrorKind::Missing))
}

fn int_array<const N: usize>(
    v: &Value,
    at: &str,
    min: i64,
    max: i64,
) -> Result<[i64; N], CaseError> {
    let wrong = || err(at, CaseErrorKind::WrongType("an array of integers"));
    let arr = v.as_array().ok_or_else(wrong)?;
    if arr.len() != N {
        return Err(err(
            at,
            CaseErrorKind::Invalid(format!("{} numbers, expected {N}", arr.len())),
        ));
    }
    let mut out = [0i64; N];
    for (i, (o, v)) in out.iter_mut().zip(arr.iter()).enumerate() {
        *o = value_int(v, &format!("{at}[{i}]"), min, max)?;
    }
    Ok(out)
}

/// `[x, y, width, height]`, width and height at least 1.
fn rect(t: &Table, at: &str, key: &str) -> Result<Option<ViewRect>, CaseError> {
    let Some(item) = t.get(key) else {
        return Ok(None);
    };
    let k = path(at, key);
    let v = item
        .as_value()
        .ok_or_else(|| err(&k, CaseErrorKind::WrongType("an array of 4 integers")))?;
    let [x, y, _, _] = int_array::<4>(v, &k, i32::MIN.into(), i32::MAX.into())?;
    let arr = v.as_array().expect("checked above");
    let w = value_int(
        arr.get(2).expect("4 numbers"),
        &format!("{k}[2]"),
        1,
        u32::MAX.into(),
    )?;
    let h = value_int(
        arr.get(3).expect("4 numbers"),
        &format!("{k}[3]"),
        1,
        u32::MAX.into(),
    )?;
    Ok(Some((x as i32, y as i32, w as u32, h as u32)))
}

/// An array of tables (`[[name]]`), each with its key path `name[i]`.
fn tables<'t>(root: &'t Table, name: &str) -> Result<Vec<(String, &'t Table)>, CaseError> {
    match root.get(name) {
        None => Ok(Vec::new()),
        Some(Item::ArrayOfTables(arr)) => Ok(arr
            .iter()
            .enumerate()
            .map(|(i, t)| (format!("{name}[{i}]"), t))
            .collect()),
        Some(_) => Err(err(
            name,
            CaseErrorKind::WrongType("an array of tables [[...]]"),
        )),
    }
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
