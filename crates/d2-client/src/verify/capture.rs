//! Reader of reference frame captures: frames of the reference game written by
//! `tools/trace-recorder/record_frames.py` as a raw JSON-lines file (formats `frames-raw-2` and
//! the older `frames-raw-1`) next to 8-bit palettized PNGs of each frame. The `scene` verify
//! cases ([`super::capture_case`]) compare D2RUST's own frames against these.
//!
//! The reader is strict: a missing or other `format`, an unknown record kind, a record or field
//! of the other format version, a missing field, a wrong type, a `seq` out of order, a key that
//! disagrees with the state it is built from, a footer whose counts disagree with the lines, or
//! a PNG that is not 8-bit color type 3 is an error naming the line or file, never a default.
//!
//! Also the two checks that need no renderer: the hashes of a frame ([`sha256_hex`],
//! [`Image::check`]) and the stability rule ([`stability`]): frames drawn from equal recorded
//! state must have equal pixels.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Cursor as IoCursor;
use std::path::{Path, PathBuf};

use d2_formats::palette::Palette;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// The raw formats this build reads. `frames-raw-1` is still accepted for the first recordings;
/// each version refuses the other's records and fields.
pub const FORMAT_1: &str = "frames-raw-1";
/// The second raw format: adds the `capture` and `celfile` records and more frame state.
pub const FORMAT_2: &str = "frames-raw-2";

/// Which raw format a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawFormat {
    /// `frames-raw-1`.
    Raw1,
    /// `frames-raw-2`.
    Raw2,
}

/// Frame record fields that only `frames-raw-2` has: refused in a `frames-raw-1` file.
const RAW2_FRAME_FIELDS: [&str; 11] = [
    "seq",
    "client_update",
    "level",
    "cursor",
    "cursor_key",
    "seed_start",
    "seed_end",
    "light",
    "light_key",
    "weather",
    "draws",
];

/// The video type a capture must have: GDI.
pub const GDI: u32 = 1;

/// SHA-256 as lowercase hex, as the recorder writes it.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The 768 palette bytes R, G, B for indices 0–255: what `palette_sha256` hashes and what the
/// PNG's PLTE holds.
pub fn palette_bytes(p: &Palette) -> Vec<u8> {
    p.colors.iter().flat_map(|c| [c.r, c.g, c.b]).collect()
}

/// A capture file error: where (`line 12: player.mode`, a file name) and
/// what is wrong.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{at}: {what}")]
pub struct CaptureError {
    /// Where: a line and field (`line 12: player.mode`) or a file name.
    pub at: String,
    /// What is wrong.
    pub what: String,
}

fn err(at: impl Into<String>, what: impl Into<String>) -> CaptureError {
    CaptureError {
        at: at.into(),
        what: what.into(),
    }
}

/// One raw capture file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raw {
    /// Which raw format the file is.
    pub format: RawFormat,
    /// The header record.
    pub header: Header,
    /// The `capture` record (`frames-raw-2` only, the line after the
    /// header).
    pub capture: Option<CaptureMeta>,
    /// `celfile` records (`frames-raw-2`), in file order.
    pub celfiles: Vec<CelFile>,
    /// `game` records: the game object the tick hook follows (hex text).
    pub games: Vec<String>,
    /// `tick` records: the server frame numbers `f`, in file order.
    pub ticks: Vec<i64>,
    /// `frame` records, in file order (refused ones included).
    pub frames: Vec<Frame>,
    /// The footer record with the counts the file must agree with.
    pub footer: Footer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// The header record of a raw capture file.
pub struct Header {
    /// The recording tool.
    pub tool: String,
    /// When the recording was made.
    pub date: String,
    /// SHA-256 of the game executable that was recorded.
    pub game_exe_sha256: String,
    /// The recorder's command-line arguments.
    pub args: Vec<String>,
}

/// The recording's settings (the `capture` record).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureMeta {
    /// Image directory name under `game/captures/` (absent with
    /// `--no-save`).
    pub images: Option<String>,
    /// `--every`: one in `every` in-game draws is captured.
    pub every: u32,
    /// `--draws-every`: one in `draws_every` captured frames carries a
    /// draw log (0: none).
    pub draws_every: u32,
    /// The names of the recorder's state key fields.
    pub state_key: Vec<String>,
}

impl CaptureMeta {
    /// Every in-game draw was captured (`--every 1`): frame `seq − 1` is the previous draw, so it
    /// is the framebuffer a frame started from.
    pub fn every_draw(&self) -> bool {
        self.every == 1
    }
}

/// One cel file load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CelFile {
    /// The cel file pointer, hex text.
    pub ptr: String,
    /// The cel file's archive path.
    pub path: String,
}

/// The footer record: totals written by the recorder, checked against the lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footer {
    /// Number of server ticks seen.
    pub ticks: i64,
    /// Free-text notes from the recorder.
    pub notes: Vec<String>,
}

/// One `frame` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// 1-based line in the raw file.
    pub line: usize,
    /// The recorder's sequence number: frames are numbered by it, never by the draw counter.
    /// `frames-raw-1` has none; its frames get their 1-based position among the frame records.
    pub seq: u32,
    /// `f`: the last server tick before this frame; `None` before the first tick of the
    /// recording.
    pub tick: Option<i64>,
    /// The video type of the recorded game (must be [`GDI`] to be captured).
    pub video_type: u32,
    /// Frame width in pixels.
    pub w: i32,
    /// Frame height in pixels.
    pub h: i32,
    /// Refused or captured content.
    pub body: FrameBody,
}

/// What a frame record holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameBody {
    /// The configuration was not GDI at 800 × 600, so nothing was captured; the text says why.
    Refused(String),
    /// A captured frame.
    Captured(Box<Captured>),
}

/// A captured frame's hashes, image name and recorded state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    /// SHA-256 of the frame's index bytes.
    pub index_sha256: String,
    /// SHA-256 of the frame's 768 palette bytes.
    pub palette_sha256: String,
    /// The game's in-game draw counter.
    pub draw: u32,
    /// PNG file name in the capture's image directory (absent with
    /// `--no-save`).
    pub image: Option<String>,
    /// The state the frame was drawn from.
    pub state: State,
    /// The draw log (`frames-raw-2`, frames selected by `--draws-every`), entries in call order,
    /// each with its `op`. A frame without one cannot be composed.
    pub draws: Option<Vec<Map<String, Value>>>,
}

/// The state a frame was drawn from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// The client player unit, if any.
    pub player: Option<Player>,
    /// The view rectangle; absent when the game had no view.
    pub view_rect: Option<[i32; 4]>,
    /// The tile origin of the camera; absent when the game had no view.
    pub tile_origin: Option<[i32; 2]>,
    /// The unit origin of the camera.
    pub unit_origin: [i32; 2],
    /// The open panel mode.
    pub open_mode: u32,
    /// The horizontal view shift.
    pub shift_x: i32,
    /// Screen shake: amplitude (unsigned), dx, dy.
    pub shake: [i64; 3],
    /// The game's screen-clear counter.
    pub clear_counter: i32,
    /// The game's resolution mode.
    pub res_mode: u32,
    /// The fields `frames-raw-2` added; `None` in a `frames-raw-1` file.
    pub more: Option<Box<State2>>,
}

/// The state `frames-raw-2` records beyond `frames-raw-1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State2 {
    /// The game's client update counter.
    pub client_update: u32,
    /// Level and act.
    pub level: Level,
    /// The cursor at frame start; absent when the frame start hook did not run for this frame.
    pub cursor: Option<Cursor>,
    /// Player random seed at frame start; absent or null without a player unit.
    pub seed_start: Option<[u32; 2]>,
    /// Player random seed at frame end; absent or null without a player unit.
    pub seed_end: Option<[u32; 2]>,
    /// Light quality and its inputs.
    pub light: Light,
    /// Weather state.
    pub weather: Weather,
}

/// Level and act.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Level {
    /// The player's level id (`None`: no player, or a null link).
    pub level_id: Option<u32>,
    /// The client's act (absent without a client act).
    pub act: Option<u8>,
    /// Environment: intensity, R, G, B.
    pub env: Option<[i64; 4]>,
}

/// The mouse cursor at frame start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// Whether the cursor was drawn.
    pub visible: u32,
    /// The cursor's state.
    pub state: i32,
    /// The cursor type.
    pub cursor_type: i32,
    /// The animation frame, 8.8 fixed point.
    pub frame: i32,
    /// Screen x.
    pub x: i32,
    /// Screen y.
    pub y: i32,
    /// The cursor's adjustment value.
    pub adj: i32,
    /// Whether the cursor holds an item.
    pub item: bool,
    /// Timer: when the animation last stepped.
    pub last_step: u32,
    /// Timer: since when the cursor has been idle.
    pub idle_since: u32,
}

/// The cursor part of the state key: drawn flag, type, drawn frame `frame >> 8`, x, y, adj, item
/// present. The timers are not part of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CursorKey {
    /// Whether the cursor was drawn.
    pub visible: u32,
    /// The cursor type.
    pub cursor_type: i32,
    /// The whole animation frame (`frame >> 8`).
    pub frame: i32,
    /// Screen x.
    pub x: i32,
    /// Screen y.
    pub y: i32,
    /// The cursor's adjustment value.
    pub adj: i32,
    /// Whether the cursor holds an item.
    pub item: bool,
}

impl Cursor {
    /// The part of the cursor that goes into the state key.
    pub fn key(&self) -> CursorKey {
        CursorKey {
            visible: self.visible,
            cursor_type: self.cursor_type,
            frame: self.frame >> 8,
            x: self.x,
            y: self.y,
            adj: self.adj,
            item: self.item,
        }
    }
}

/// Light quality and its inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Light {
    /// Light quality, 0–2: the state key's `light_key`.
    pub quality: i32,
    /// The game's draw rate setting.
    pub draw_rate: i32,
    /// First light option flag.
    pub opt_a: u32,
    /// Second light option flag.
    pub opt_b: u32,
    /// The game's render kind.
    pub render_kind: u32,
}

/// Weather state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Weather {
    /// Rain flag.
    pub rain: u32,
    /// Snow flag.
    pub snow: u32,
    /// Lightning state.
    pub lightning: i32,
    /// Lightning flash state.
    pub flash: i32,
    /// Weather update counter.
    pub update: u32,
}

/// The client player unit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Player {
    /// The unit's type.
    pub unit_type: u32,
    /// The unit's mode (animation state).
    pub mode: u32,
    /// A further per-unit value recorded as is; what it means isn't established here.
    pub cur: i32,
    /// The path's fixed-point position, present for a player unit with a path.
    pub fixed: Option<[u32; 2]>,
    /// The path's client position, present with `fixed`.
    pub client: Option<[i32; 2]>,
}

impl Frame {
    /// The captured content, or `None` for a refused frame.
    pub fn captured(&self) -> Option<&Captured> {
        match &self.body {
            FrameBody::Captured(c) => Some(c),
            FrameBody::Refused(_) => None,
        }
    }
}

/// Reads a raw capture file (`frames-raw-1` or `frames-raw-2`).
pub fn read_raw(path: &Path) -> Result<Raw, CaptureError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| err(path.display().to_string(), e.to_string()))?;
    parse_raw(&text).map_err(|e| err(format!("{}: {}", path.display(), e.at), e.what))
}

/// Parses the text of a raw capture file.
pub fn parse_raw(text: &str) -> Result<Raw, CaptureError> {
    let mut header = None;
    let mut format = RawFormat::Raw1;
    let mut capture = None;
    let mut footer = None;
    let (mut games, mut ticks, mut frames) = (Vec::new(), Vec::new(), Vec::new());
    let mut celfiles = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        let at = format!("line {n}");
        if footer.is_some() {
            return Err(err(at, "a record after the footer"));
        }
        let value: Value =
            serde_json::from_str(line).map_err(|e| err(&at, format!("JSON: {e}")))?;
        let rec = value
            .as_object()
            .ok_or_else(|| err(&at, "a record is a JSON object"))?;
        let r = Rec { at: &at, rec };
        let kind = r.str("k")?;
        if header.is_none() && kind != "header" {
            return Err(err(
                &at,
                format!("first record is {kind:?}, not the header"),
            ));
        }
        // In `frames-raw-2` the `capture` record follows the header.
        if format == RawFormat::Raw2 && n == 2 && kind != "capture" {
            return Err(err(
                &at,
                format!("second record is {kind:?}, not the {FORMAT_2} capture record"),
            ));
        }
        match kind {
            "header" => {
                if header.is_some() {
                    return Err(err(at, "a second header"));
                }
                // Version first: an unknown format is reported before anything it may have
                // changed.
                format = match r.str("format")? {
                    FORMAT_1 => RawFormat::Raw1,
                    FORMAT_2 => RawFormat::Raw2,
                    other => {
                        return Err(err(
                            &at,
                            format!(
                                "format {other:?} is not supported (this build reads {FORMAT_1:?} and {FORMAT_2:?})"
                            ),
                        ))
                    }
                };
                let args = r
                    .get("args")?
                    .as_array()
                    .ok_or_else(|| r.wrong("args", "an array of strings"))?
                    .iter()
                    .map(|a| {
                        a.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| r.wrong("args", "an array of strings"))
                    })
                    .collect::<Result<_, _>>()?;
                header = Some(Header {
                    tool: r.str("tool")?.to_owned(),
                    date: r.str("date")?.to_owned(),
                    game_exe_sha256: r.str("game_exe_sha256")?.to_owned(),
                    args,
                });
            }
            "capture" | "celfile" if format == RawFormat::Raw1 => {
                return Err(err(
                    &at,
                    format!("a {FORMAT_2} {kind:?} record in a {FORMAT_1} file"),
                ))
            }
            "capture" => {
                if n != 2 {
                    return Err(err(at, "the capture record is not the second line"));
                }
                capture = Some(capture_meta(&r)?);
            }
            "celfile" => celfiles.push(CelFile {
                ptr: r.str("ptr")?.to_owned(),
                path: r.str("path")?.to_owned(),
            }),
            "game" => games.push(r.str("g")?.to_owned()),
            "tick" => ticks.push(r.int("f", i64::MIN, i64::MAX)?),
            "frame" => {
                let want = frames.len() as u32 + 1;
                frames.push(frame(&r, n, format, want)?)
            }
            "footer" => {
                let counts = r
                    .get("counts")?
                    .as_object()
                    .ok_or_else(|| r.wrong("counts", "an object"))?;
                // The recorder counts every emitted record by kind; the
                // lines must agree (a cut or edited file is caught here).
                let seen = [
                    ("game", games.len()),
                    ("tick", ticks.len()),
                    ("frame", frames.len()),
                    ("capture", usize::from(capture.is_some())),
                    ("celfile", celfiles.len()),
                ];
                for (k, v) in counts {
                    let want = v
                        .as_u64()
                        .ok_or_else(|| r.wrong(&format!("counts.{k}"), "an integer"))?;
                    let got = seen
                        .iter()
                        .find(|(name, _)| name == k)
                        .map(|&(_, c)| c)
                        .ok_or_else(|| err(&at, format!("counts.{k}: unknown record kind")))?;
                    if want != got as u64 {
                        return Err(err(
                            &at,
                            format!("counts.{k} = {want}, the file has {got} such lines"),
                        ));
                    }
                }
                for (name, c) in seen {
                    if c > 0 && !counts.contains_key(name) {
                        return Err(err(&at, format!("counts has no {name} ({c} lines)")));
                    }
                }
                let notes = r
                    .get("notes")?
                    .as_array()
                    .ok_or_else(|| r.wrong("notes", "an array of strings"))?
                    .iter()
                    .map(|a| {
                        a.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| r.wrong("notes", "an array of strings"))
                    })
                    .collect::<Result<_, _>>()?;
                let n_ticks = r.int("ticks", 0, i64::MAX)?;
                if n_ticks != ticks.len() as i64 {
                    return Err(err(
                        &at,
                        format!("ticks = {n_ticks}, the file has {} tick lines", ticks.len()),
                    ));
                }
                footer = Some(Footer {
                    ticks: n_ticks,
                    notes,
                });
            }
            other => return Err(err(&at, format!("unknown record kind {other:?}"))),
        }
    }
    let header = header.ok_or_else(|| err("line 1", "empty file (no header)"))?;
    let footer = footer.ok_or_else(|| err("end of file", "no footer (the recording was cut)"))?;
    if format == RawFormat::Raw2 && capture.is_none() {
        return Err(err("line 2", format!("no {FORMAT_2} capture record")));
    }
    Ok(Raw {
        format,
        header,
        capture,
        celfiles,
        games,
        ticks,
        frames,
        footer,
    })
}

fn capture_meta(r: &Rec<'_>) -> Result<CaptureMeta, CaptureError> {
    let images = match r.get("images")? {
        Value::Null => None,
        _ => Some(r.str("images")?.to_owned()),
    };
    let state_key = r
        .get("state_key")?
        .as_array()
        .ok_or_else(|| r.wrong("state_key", "an array of strings"))?
        .iter()
        .map(|a| {
            a.as_str()
                .map(str::to_owned)
                .ok_or_else(|| r.wrong("state_key", "an array of strings"))
        })
        .collect::<Result<_, _>>()?;
    Ok(CaptureMeta {
        images,
        every: r.int("every", 1, u32::MAX.into())? as u32,
        draws_every: r.int("draws_every", 0, u32::MAX.into())? as u32,
        state_key,
    })
}

fn frame(
    r: &Rec<'_>,
    line: usize,
    format: RawFormat,
    want_seq: u32,
) -> Result<Frame, CaptureError> {
    let seq = match format {
        RawFormat::Raw1 => {
            if let Some(k) = RAW2_FRAME_FIELDS.iter().find(|k| r.rec.contains_key(**k)) {
                return Err(err(
                    r.at,
                    format!("{k}: a {FORMAT_2} field in a {FORMAT_1} file"),
                ));
            }
            want_seq
        }
        RawFormat::Raw2 => {
            let seq = r.int("seq", 0, u32::MAX.into())? as u32;
            if seq != want_seq {
                return Err(err(
                    r.at,
                    format!("seq {seq}: frame records are numbered 1, 2, … (expected {want_seq})"),
                ));
            }
            seq
        }
    };
    let tick = match r.get("f")? {
        Value::Null => None,
        _ => Some(r.int("f", i64::MIN, i64::MAX)?),
    };
    let video_type = r.int("video_type", 0, u32::MAX.into())? as u32;
    let w = r.int("w", i32::MIN.into(), i32::MAX.into())? as i32;
    let h = r.int("h", i32::MIN.into(), i32::MAX.into())? as i32;
    let body = if r.rec.contains_key("refused") {
        FrameBody::Refused(r.str("refused")?.to_owned())
    } else {
        if video_type != GDI || w <= 0 || h <= 0 {
            return Err(err(
                r.at,
                format!("a captured frame with video type {video_type} at {w}x{h} (only GDI frames with a size are captured)"),
            ));
        }
        let image = match r.rec.get("image") {
            None => None,
            Some(_) => Some(r.str("image")?.to_owned()),
        };
        let draws = match r.rec.get("draws") {
            None => None,
            Some(v) => Some(draw_log(r, v)?),
        };
        FrameBody::Captured(Box::new(Captured {
            index_sha256: r.hash("index_sha256")?,
            palette_sha256: r.hash("palette_sha256")?,
            draw: r.int("draw", 0, u32::MAX.into())? as u32,
            image,
            state: state(r, format)?,
            draws,
        }))
    };
    Ok(Frame {
        line,
        seq,
        tick,
        video_type,
        w,
        h,
        body,
    })
}

/// The draw log: an array of objects, each naming its `op`.
fn draw_log(r: &Rec<'_>, v: &Value) -> Result<Vec<Map<String, Value>>, CaptureError> {
    let arr = v
        .as_array()
        .ok_or_else(|| r.wrong("draws", "an array of draw entries"))?;
    arr.iter()
        .enumerate()
        .map(|(i, e)| {
            let o = e
                .as_object()
                .ok_or_else(|| r.wrong(&format!("draws[{i}]"), "an object"))?;
            o.get("op")
                .and_then(Value::as_str)
                .ok_or_else(|| r.wrong(&format!("draws[{i}].op"), "a string"))?;
            Ok(o.clone())
        })
        .collect()
}

fn state(r: &Rec<'_>, format: RawFormat) -> Result<State, CaptureError> {
    let i32s = |key: &str| r.ints::<2>(key, i32::MIN.into(), i32::MAX.into());
    let player = match r.rec.get("player") {
        None => None,
        Some(v) => {
            let p = Rec {
                at: &format!("{}: player", r.at),
                rec: v
                    .as_object()
                    .ok_or_else(|| r.wrong("player", "an object"))?,
            };
            let fixed = match p.rec.get("fixed") {
                None => None,
                Some(_) => Some(p.ints::<2>("fixed", 0, u32::MAX.into())?.map(|v| v as u32)),
            };
            let client = match p.rec.get("client") {
                None => None,
                Some(_) => Some(
                    p.ints::<2>("client", i32::MIN.into(), i32::MAX.into())?
                        .map(|v| v as i32),
                ),
            };
            if fixed.is_some() != client.is_some() {
                return Err(p.wrong("fixed", "fixed and client together"));
            }
            Some(Player {
                unit_type: p.int("type", 0, u32::MAX.into())? as u32,
                mode: p.int("mode", 0, u32::MAX.into())? as u32,
                cur: p.int("cur", i32::MIN.into(), i32::MAX.into())? as i32,
                fixed,
                client,
            })
        }
    };
    let view_rect = match r.rec.get("view_rect") {
        None => None,
        Some(_) => Some(
            r.ints::<4>("view_rect", i32::MIN.into(), i32::MAX.into())?
                .map(|v| v as i32),
        ),
    };
    let tile_origin = match r.rec.get("tile_origin") {
        None => None,
        Some(_) => Some(i32s("tile_origin")?.map(|v| v as i32)),
    };
    let shake = r.ints::<3>("shake", i32::MIN.into(), u32::MAX.into())?;
    // The amplitude is read unsigned, dx and dy signed.
    if shake[0] < 0 || shake[1..].iter().any(|&v| v > i32::MAX.into()) {
        return Err(r.wrong("shake", "[u32, i32, i32]"));
    }
    Ok(State {
        player,
        view_rect,
        tile_origin,
        unit_origin: i32s("unit_origin")?.map(|v| v as i32),
        open_mode: r.int("open_mode", 0, u32::MAX.into())? as u32,
        shift_x: r.int("shift_x", i32::MIN.into(), i32::MAX.into())? as i32,
        shake,
        clear_counter: r.int("clear_counter", i32::MIN.into(), i32::MAX.into())? as i32,
        res_mode: r.int("res_mode", 0, u32::MAX.into())? as u32,
        more: match format {
            RawFormat::Raw1 => None,
            RawFormat::Raw2 => Some(Box::new(state2(r)?)),
        },
    })
}

const I32: (i64, i64) = (i32::MIN as i64, i32::MAX as i64);
const U32: (i64, i64) = (0, u32::MAX as i64);

/// The `frames-raw-2` state, with the recorded `cursor_key` and `light_key` checked against the
/// state they are built from.
fn state2(r: &Rec<'_>) -> Result<State2, CaptureError> {
    let object = |key: &str| {
        r.get(key)?
            .as_object()
            .ok_or_else(|| r.wrong(key, "an object"))
    };
    let at = format!("{}: level", r.at);
    let l = Rec {
        at: &at,
        rec: object("level")?,
    };
    let level = Level {
        level_id: match l.rec.get("level_id") {
            None | Some(Value::Null) => None,
            Some(_) => Some(l.int("level_id", U32.0, U32.1)? as u32),
        },
        act: match l.rec.get("act") {
            None => None,
            Some(_) => Some(l.int("act", 0, 255)? as u8),
        },
        env: match l.rec.get("env") {
            None => None,
            Some(_) => {
                let e = l.ints::<4>("env", I32.0, I32.1)?;
                if e[1..].iter().any(|&c| !(0..=255).contains(&c)) {
                    return Err(l.wrong("env", "[intensity, r, g, b] with bytes r, g, b"));
                }
                Some(e)
            }
        },
    };
    let cursor = match r.rec.get("cursor") {
        None => None,
        Some(_) => {
            let at = format!("{}: cursor", r.at);
            let c = Rec {
                at: &at,
                rec: object("cursor")?,
            };
            let i = |k: &str| c.int(k, I32.0, I32.1).map(|v| v as i32);
            let u = |k: &str| c.int(k, U32.0, U32.1).map(|v| v as u32);
            Some(Cursor {
                visible: u("visible")?,
                state: i("state")?,
                cursor_type: i("type")?,
                frame: i("frame")?,
                x: i("x")?,
                y: i("y")?,
                adj: i("adj")?,
                item: c
                    .get("item")?
                    .as_bool()
                    .ok_or_else(|| c.wrong("item", "a boolean"))?,
                last_step: u("last_step")?,
                idle_since: u("idle_since")?,
            })
        }
    };
    // cursor_key: present exactly with the cursor, equal to its key.
    match (cursor, r.rec.get("cursor_key")) {
        (None, None) => {}
        (Some(c), Some(v)) => {
            let k = c.key();
            let want =
                serde_json::json!([k.visible, k.cursor_type, k.frame, k.x, k.y, k.adj, k.item]);
            if *v != want {
                return Err(r.wrong("cursor_key", &format!("{want} (the key of `cursor`)")));
            }
        }
        (Some(_), None) => return Err(err(r.at, "cursor_key: missing")),
        (None, Some(_)) => return Err(err(r.at, "cursor_key without a cursor")),
    }
    let seed = |key: &str| -> Result<Option<[u32; 2]>, CaptureError> {
        match r.rec.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(_) => Ok(Some(r.ints::<2>(key, U32.0, U32.1)?.map(|v| v as u32))),
        }
    };
    let at = format!("{}: light", r.at);
    let lr = Rec {
        at: &at,
        rec: object("light")?,
    };
    let light = Light {
        quality: lr.int("quality", I32.0, I32.1)? as i32,
        draw_rate: lr.int("draw_rate", I32.0, I32.1)? as i32,
        opt_a: lr.int("opt_a", U32.0, U32.1)? as u32,
        opt_b: lr.int("opt_b", U32.0, U32.1)? as u32,
        render_kind: lr.int("render_kind", U32.0, U32.1)? as u32,
    };
    let light_key = r.int("light_key", I32.0, I32.1)? as i32;
    if light_key != light.quality {
        return Err(r.wrong("light_key", &format!("{} (light.quality)", light.quality)));
    }
    let at = format!("{}: weather", r.at);
    let w = Rec {
        at: &at,
        rec: object("weather")?,
    };
    let weather = Weather {
        rain: w.int("rain", U32.0, U32.1)? as u32,
        snow: w.int("snow", U32.0, U32.1)? as u32,
        lightning: w.int("lightning", I32.0, I32.1)? as i32,
        flash: w.int("flash", I32.0, I32.1)? as i32,
        update: w.int("update", U32.0, U32.1)? as u32,
    };
    Ok(State2 {
        client_update: r.int("client_update", U32.0, U32.1)? as u32,
        level,
        cursor,
        seed_start: seed("seed_start")?,
        seed_end: seed("seed_end")?,
        light,
        weather,
    })
}

/// Typed field access on one record, errors naming the line and key.
struct Rec<'a> {
    at: &'a str,
    rec: &'a Map<String, Value>,
}

impl Rec<'_> {
    fn wrong(&self, key: &str, want: &str) -> CaptureError {
        err(self.at, format!("{key}: expected {want}"))
    }

    fn get(&self, key: &str) -> Result<&Value, CaptureError> {
        self.rec
            .get(key)
            .ok_or_else(|| err(self.at, format!("{key}: missing")))
    }

    fn str(&self, key: &str) -> Result<&str, CaptureError> {
        self.get(key)?
            .as_str()
            .ok_or_else(|| self.wrong(key, "a string"))
    }

    fn hash(&self, key: &str) -> Result<String, CaptureError> {
        let s = self.str(key)?;
        if s.len() != 64
            || !s
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(self.wrong(key, "64 lowercase hex digits"));
        }
        Ok(s.to_owned())
    }

    fn int_of(&self, key: &str, v: &Value, min: i64, max: i64) -> Result<i64, CaptureError> {
        let v = v.as_i64().ok_or_else(|| self.wrong(key, "an integer"))?;
        if !(min..=max).contains(&v) {
            return Err(err(
                self.at,
                format!("{key}: {v} is out of range {min}..={max}"),
            ));
        }
        Ok(v)
    }

    fn int(&self, key: &str, min: i64, max: i64) -> Result<i64, CaptureError> {
        self.int_of(key, self.get(key)?, min, max)
    }

    fn ints<const N: usize>(
        &self,
        key: &str,
        min: i64,
        max: i64,
    ) -> Result<[i64; N], CaptureError> {
        let arr = self
            .get(key)?
            .as_array()
            .filter(|a| a.len() == N)
            .ok_or_else(|| self.wrong(key, &format!("an array of {N} integers")))?;
        let mut out = [0; N];
        for (o, v) in out.iter_mut().zip(arr) {
            *o = self.int_of(key, v, min, max)?;
        }
        Ok(out)
    }
}

/// A capture PNG read back: the index bytes and the 768 palette bytes,
/// unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// `width × height` bytes, row-major, top row first.
    pub indices: Vec<u8>,
    /// 768 bytes R, G, B.
    pub palette: Vec<u8>,
}

impl Image {
    /// The integrity check of a capture: the PNG's size, index hash and
    /// palette hash equal the frame record's. A failure means the PNG and the raw file disagree
    /// (an edited or mixed-up file).
    pub fn check(&self, frame: &Frame, c: &Captured) -> Result<(), String> {
        if (self.width as i64, self.height as i64) != (frame.w.into(), frame.h.into()) {
            return Err(format!(
                "PNG is {}x{}, the record {}x{}",
                self.width, self.height, frame.w, frame.h
            ));
        }
        let got = sha256_hex(&self.indices);
        if got != c.index_sha256 {
            return Err(format!(
                "PNG index hash {got} differs from the record's index_sha256 {}",
                c.index_sha256
            ));
        }
        let got = sha256_hex(&self.palette);
        if got != c.palette_sha256 {
            return Err(format!(
                "PNG palette hash {got} differs from the record's palette_sha256 {}",
                c.palette_sha256
            ));
        }
        Ok(())
    }
}

/// Decodes a capture PNG: 8-bit, color type 3 (palettized), 256 PLTE
/// entries; the indices are returned as stored (no expansion).
pub fn decode_png(bytes: &[u8]) -> Result<Image, String> {
    let mut decoder = png::Decoder::new(IoCursor::new(bytes));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(|e| format!("PNG: {e}"))?;
    let info = reader.info();
    if info.color_type != png::ColorType::Indexed || info.bit_depth != png::BitDepth::Eight {
        return Err(format!(
            "PNG is {:?} {:?}, a capture is 8-bit palettized (color type 3)",
            info.bit_depth, info.color_type
        ));
    }
    if info.interlaced {
        return Err("PNG is interlaced; a capture is not".into());
    }
    let palette = info.palette.as_ref().ok_or("PNG has no PLTE")?.to_vec();
    if palette.len() != 768 {
        return Err(format!(
            "PLTE has {} bytes, a capture has 768",
            palette.len()
        ));
    }
    let (width, height) = (info.width, info.height);
    let size = reader.output_buffer_size().ok_or("PNG frame too large")?;
    let mut buf = vec![0; size];
    let out = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("PNG: {e}"))?;
    let len = width as usize * height as usize;
    if out.line_size != width as usize || out.buffer_size() != len {
        return Err(format!(
            "PNG rows are {} bytes for width {width}",
            out.line_size
        ));
    }
    buf.truncate(len);
    Ok(Image {
        width,
        height,
        indices: buf,
        palette,
    })
}

/// Reads the PNG of a captured frame from the capture's image directory.
pub fn read_image(dir: &Path, c: &Captured) -> Result<(PathBuf, Image), String> {
    let name = c
        .image
        .as_deref()
        .ok_or("the record has no image (recorded with --no-save)")?;
    let path = dir.join(name);
    let bytes = std::fs::read(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let image = decode_png(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((path, image))
}

/// The state key: frames with equal keys were drawn from equal recorded state. Same fields as
/// the recorder's key: player record
/// (with the 8.8 frame), tile origin, unit origin, shake, open mode,
/// palette hash and, from `frames-raw-2`, `cursor_key`, `light_key` and
/// level (`None` in a `frames-raw-1` file, whose key had none of them).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StateKey {
    /// The client player record.
    pub player: Option<Player>,
    /// The camera tile origin.
    pub tile_origin: Option<[i32; 2]>,
    /// The camera unit origin.
    pub unit_origin: [i32; 2],
    /// Screen shake: amplitude, dx, dy.
    pub shake: [i64; 3],
    /// The open panel mode.
    pub open_mode: u32,
    /// Hash of the palette.
    pub palette_sha256: String,
    /// The cursor's key part (`frames-raw-2`).
    pub cursor_key: Option<CursorKey>,
    /// The light quality (`frames-raw-2`).
    pub light_key: Option<i32>,
    /// Level and act (`frames-raw-2`).
    pub level: Option<Level>,
}

impl StateKey {
    /// The key of a captured frame's recorded state.
    pub fn of(c: &Captured) -> Self {
        let s = &c.state;
        let more = s.more.as_deref();
        StateKey {
            player: s.player.clone(),
            tile_origin: s.tile_origin,
            unit_origin: s.unit_origin,
            shake: s.shake,
            open_mode: s.open_mode,
            palette_sha256: c.palette_sha256.clone(),
            cursor_key: more.and_then(|m| m.cursor).map(|c| c.key()),
            light_key: more.map(|m| m.light.quality),
            level: more.map(|m| m.level.clone()),
        }
    }
}

/// One state group: the frames (`seq`) with that key and the index hashes
/// seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The shared state key.
    pub key: StateKey,
    /// The `seq` of each frame in the group.
    pub seqs: Vec<u32>,
    /// The distinct index hashes seen (more than one means the frames differ).
    pub hashes: BTreeSet<String>,
}

/// The result of the stability rule: frames with equal state keys must have equal pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stability {
    /// All groups, in key order.
    pub groups: Vec<Group>,
}

impl Stability {
    /// Groups seen at least twice.
    pub fn repeated(&self) -> usize {
        self.groups.iter().filter(|g| g.seqs.len() >= 2).count()
    }

    /// Groups whose frames differ.
    pub fn differing(&self) -> impl Iterator<Item = &Group> {
        self.groups.iter().filter(|g| g.hashes.len() > 1)
    }

    /// The rule holds: no group has differing frames and at least two keys were seen at least
    /// twice each.
    pub fn holds(&self) -> bool {
        self.differing().next().is_none() && self.repeated() >= 2
    }
}

impl fmt::Display for Stability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} state groups, {} seen at least twice, {} with differing frames",
            self.groups.len(),
            self.repeated(),
            self.differing().count()
        )
    }
}

/// Groups captured frames by [`StateKey`]. Each item is a frame's `seq`, its record and its
/// index hash (the record's, or a re-hash of its PNG).
pub fn stability<'a>(frames: impl IntoIterator<Item = (u32, &'a Captured, String)>) -> Stability {
    let mut groups: BTreeMap<StateKey, Group> = BTreeMap::new();
    for (seq, c, hash) in frames {
        let key = StateKey::of(c);
        let g = groups.entry(key.clone()).or_insert_with(|| Group {
            key,
            seqs: Vec::new(),
            hashes: BTreeSet::new(),
        });
        g.seqs.push(seq);
        g.hashes.insert(hash);
    }
    Stability {
        groups: groups.into_values().collect(),
    }
}

/// Ticks shared by more than one captured frame (a draw with no server tick before it), as
/// `(tick, seqs)`.
pub fn repeated_ticks(frames: &[Frame]) -> Vec<(i64, Vec<u32>)> {
    let mut by_tick: BTreeMap<i64, Vec<u32>> = BTreeMap::new();
    for f in frames {
        if let (Some(t), Some(_)) = (f.tick, f.captured()) {
            by_tick.entry(t).or_default().push(f.seq);
        }
    }
    by_tick.into_iter().filter(|(_, d)| d.len() > 1).collect()
}
