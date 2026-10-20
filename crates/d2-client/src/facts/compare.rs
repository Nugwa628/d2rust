//! `facts-compare`: the first difference between two fact sets, cause before effect.
//!
//! The two sides are the "original" set (the reference) and the "d2rust" set (D2RUST's export).
//! Stages are checked in this order: frame inputs, sprites, draws, frame outputs. A cell that
//! either side marks as unmeasured (`?`) never counts as a difference; it is only tallied, and a
//! run with no difference but some unmeasured cells is `Partial` rather than `Match`. Facts
//! files are produced by [`super::export`] and parsed by the helpers in [`super`].

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use super::{
    check_frame_keys, read, FactsError, Role, Table, DRAWS_FILE, DRAW_COLUMNS, FRAME_COLUMNS,
    FRAME_FILE, FRAME_KEYS, SPRITES_FILE, SPRITE_COLUMNS, UNKNOWN,
};

/// Columns and keys never compared: draw index, call-site tag and frame sequence number.
const INFO: [&str; 3] = ["i", "at", "seq"];

/// The three files of one side.
#[derive(Debug, Clone)]
pub struct FactSet {
    /// `draws.tsv`.
    pub draws: Table,
    /// `frame.tsv`.
    pub frame: Table,
    /// `None`: no `sprites.tsv` found, so sprite sizes count as unmeasured.
    pub sprites: Option<Table>,
}

impl FactSet {
    /// Reads a scene or dump directory. `sprites_fallback` is tried when
    /// the directory has no `sprites.tsv` (for the reference side, `compare_dirs` passes
    /// `../../sprites.tsv`).
    pub fn read(dir: &Path, sprites_fallback: Option<&Path>) -> Result<FactSet, FactsError> {
        if !dir.is_dir() {
            return Err(FactsError::Io {
                path: dir.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such directory"),
            });
        }
        let draws = read(dir, DRAWS_FILE, &DRAW_COLUMNS)?;
        let frame = read(dir, FRAME_FILE, &FRAME_COLUMNS)?;
        check_frame_keys(&dir.join(FRAME_FILE).display().to_string(), &frame)?;
        let sprites = [Some(dir), sprites_fallback]
            .into_iter()
            .flatten()
            .find(|d| d.join(SPRITES_FILE).is_file())
            .map(|d| read(d, SPRITES_FILE, &SPRITE_COLUMNS))
            .transpose()?;
        Ok(FactSet {
            draws,
            frame,
            sprites,
        })
    }
}

/// Where the first difference is, in check order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The state the frame was drawn from (`frame.tsv` input keys).
    FrameInput,
    /// Sprite sizes and offsets (`sprites.tsv`).
    Sprites,
    /// The draw list (`draws.tsv`).
    Draws,
    /// What the frame presented (`frame.tsv` output keys).
    FrameOutput,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Stage::FrameInput => "frame.tsv (inputs)",
            Stage::Sprites => "sprites.tsv",
            Stage::Draws => "draws.tsv",
            Stage::FrameOutput => "frame.tsv (outputs)",
        })
    }
}

/// The first difference: stage, row (0-based data row of that file),
/// column, and both full rows (`None`: the row is missing on that side).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// Which file/stage differs.
    pub stage: Stage,
    /// 0-based data row within that file.
    pub row: usize,
    /// The differing column (or `frame.tsv` key).
    pub column: String,
    /// The reference side's row.
    pub original: Option<Vec<String>>,
    /// D2RUST's row.
    pub d2rust: Option<Vec<String>>,
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let row = |r: &Option<Vec<String>>| r.as_ref().map_or("<none>".into(), |r| r.join("\t"));
        writeln!(
            f,
            "first difference: {}, row {}, column {}",
            self.stage, self.row, self.column
        )?;
        writeln!(f, "  original: {}", row(&self.original))?;
        write!(f, "  D2RUST:   {}", row(&self.d2rust))
    }
}

/// The compare's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// No difference and nothing unmeasured.
    Match,
    /// A first difference was found.
    Diverged(Difference),
    /// No difference; unmeasured cells per column (or `sprites.tsv`).
    Partial(BTreeMap<String, usize>),
}

impl Outcome {
    /// Process exit code: 0 match, 1 diverged, 2 partial (3, error, is the caller's).
    pub fn exit_code(&self) -> i32 {
        match self {
            Outcome::Match => 0,
            Outcome::Diverged(_) => 1,
            Outcome::Partial(_) => 2,
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Outcome::Match => f.write_str("MATCH"),
            Outcome::Diverged(d) => write!(f, "DIVERGED\n{d}"),
            Outcome::Partial(u) => {
                write!(f, "PARTIAL: no difference; unmeasured:")?;
                for (k, n) in u {
                    write!(f, " {k} {n};")?;
                }
                Ok(())
            }
        }
    }
}

struct Walk<'a> {
    ignore: &'a [String],
    unmeasured: BTreeMap<String, usize>,
}

impl Walk<'_> {
    fn skipped(&self, column: &str) -> bool {
        INFO.contains(&column) || self.ignore.iter().any(|c| c == column)
    }

    /// Whether `a` and `b` differ; a `?` on either side is counted as unmeasured, not a difference.
    fn differ(&mut self, column: &str, a: &str, b: &str) -> bool {
        if self.skipped(column) {
            return false;
        }
        if a == UNKNOWN || b == UNKNOWN {
            *self.unmeasured.entry(column.to_owned()).or_default() += 1;
            return false;
        }
        a != b
    }

    fn frame_stage(&mut self, o: &Table, d: &Table, role: Role) -> Option<Difference> {
        let stage = match role {
            Role::Output => Stage::FrameOutput,
            _ => Stage::FrameInput,
        };
        for (i, (key, r)) in FRAME_KEYS.iter().enumerate() {
            if *r == role && self.differ(key, &o.rows[i][1], &d.rows[i][1]) {
                return Some(Difference {
                    stage,
                    row: i,
                    column: (*key).to_owned(),
                    original: Some(o.rows[i].clone()),
                    d2rust: Some(d.rows[i].clone()),
                });
            }
        }
        None
    }

    fn sprites(
        &mut self,
        o: Option<&Table>,
        d: Option<&Table>,
        o_draws: &Table,
    ) -> Option<Difference> {
        let (Some(o), Some(d)) = (o, d) else {
            *self
                .unmeasured
                .entry(super::SPRITES_FILE.into())
                .or_default() += 1;
            return None;
        };
        let key = |r: &Vec<String>| (r[0].clone(), r[1].clone(), r[2].clone());
        let by_key: BTreeMap<_, _> = o.rows.iter().map(|r| (key(r), r)).collect();
        for (n, row) in d.rows.iter().enumerate() {
            let Some(orig) = by_key.get(&key(row)) else {
                // The reference never rasterizes a minimap cel, so it has no sprite row for it.
                if is_maximap_file(&row[0]) {
                    continue;
                }
                // A cel the reference drew through its direction fallback has no sprite key;
                // its draw rows carry the size it read, so compare against those instead.
                let drawn = fallback_sizes(o_draws, row);
                if drawn.is_empty() {
                    *self.unmeasured.entry("sprite rows".into()).or_default() += 1;
                }
                for size in drawn {
                    if let Some(c) = (0..4).find(|&c| size[c] != row[3 + c]) {
                        let mut orig = row[..3].to_vec();
                        orig.extend(size.iter().map(|v| (*v).to_owned()));
                        return Some(Difference {
                            stage: Stage::Sprites,
                            row: n,
                            column: SPRITE_COLUMNS[3 + c].to_owned(),
                            original: Some(orig),
                            d2rust: Some(row.clone()),
                        });
                    }
                }
                continue;
            };
            for (c, name) in SPRITE_COLUMNS.iter().enumerate().skip(3) {
                if self.differ(name, &orig[c], &row[c]) {
                    return Some(Difference {
                        stage: Stage::Sprites,
                        row: n,
                        column: (*name).to_owned(),
                        original: Some((*orig).clone()),
                        d2rust: Some(row.clone()),
                    });
                }
            }
        }
        None
    }

    fn draws(&mut self, o: &Table, d: &Table) -> Option<Difference> {
        let n = o.rows.len().max(d.rows.len());
        for i in 0..n {
            let (a, b) = (o.rows.get(i), d.rows.get(i));
            let (Some(a), Some(b)) = (a, b) else {
                return Some(Difference {
                    stage: Stage::Draws,
                    row: i,
                    column: "<row>".into(),
                    original: a.cloned(),
                    d2rust: b.cloned(),
                });
            };
            let maximap = is_maximap_row(a);
            for (c, name) in DRAW_COLUMNS.iter().enumerate() {
                if maximap && a[c] == UNKNOWN {
                    continue;
                }
                if self.differ(name, &a[c], &b[c]) {
                    return Some(Difference {
                        stage: Stage::Draws,
                        row: i,
                        column: (*name).to_owned(),
                        original: Some(a.clone()),
                        d2rust: Some(b.clone()),
                    });
                }
            }
        }
        None
    }
}

/// Whether a reference `draws.tsv` row is a `maximap.dc6` cel drawn with `CelDrawClipped`. The
/// reference never rasterizes these, so their `?` cells are left out and not counted.
fn is_maximap_row(row: &[String]) -> bool {
    row[draw_column("op")] == "CelDrawClipped" && is_maximap_file(&row[draw_column("file")])
}

/// `w h xoff yoff` of every reference cel row of the sprite's (`file`, `dir`, `frame`) whose
/// four cells are measured.
fn fallback_sizes<'t>(o_draws: &'t Table, sprite: &[String]) -> Vec<[&'t str; 4]> {
    let key = [
        draw_column("file"),
        draw_column("dir"),
        draw_column("frame"),
    ];
    let size = [
        draw_column("w"),
        draw_column("h"),
        draw_column("xoff"),
        draw_column("yoff"),
    ];
    o_draws
        .rows
        .iter()
        .filter(|r| key.iter().zip(sprite).all(|(&k, s)| r[k] == *s))
        .map(|r| size.map(|c| r[c].as_str()))
        .filter(|s| s.iter().all(|v| *v != UNKNOWN && *v != super::NA))
        .collect()
}

fn is_maximap_file(file: &str) -> bool {
    file.to_ascii_lowercase().ends_with("maximap.dc6")
}

/// Compares two parsed fact sets stage by stage. `ignore` names columns or `frame.tsv` keys left
/// out.
pub fn compare(original: &FactSet, d2rust: &FactSet, ignore: &[String]) -> Outcome {
    let mut w = Walk {
        ignore,
        unmeasured: BTreeMap::new(),
    };
    let first = w
        .frame_stage(&original.frame, &d2rust.frame, Role::Input)
        .or_else(|| {
            w.sprites(
                original.sprites.as_ref(),
                d2rust.sprites.as_ref(),
                &original.draws,
            )
        })
        .or_else(|| w.draws(&original.draws, &d2rust.draws))
        .or_else(|| w.frame_stage(&original.frame, &d2rust.frame, Role::Output));
    match first {
        Some(d) => Outcome::Diverged(d),
        None if w.unmeasured.is_empty() => Outcome::Match,
        None => Outcome::Partial(w.unmeasured),
    }
}

/// The call-site address range of the reference's pass 9 draw calls (weather, between the
/// particle draw and the weather update). A reference row whose `at` falls in it is a weather
/// row.
const PASS9_AT: std::ops::Range<u32> = 0x0047_3470..0x0047_3F50;

/// The `at` tag D2RUST's exporter writes on a pass 9 (weather) row.
pub const PASS9_TAG: &str = "pass9";

/// The call-site address range of the reference's pass 4 pool draw (the environment pools). A
/// reference row whose `at` falls in it is a pool cel.
const POOLS_AT: std::ops::Range<u32> = 0x0047_3A70..0x0047_3C00;

/// The `at` tag D2RUST's exporter writes on a pass 4 pool cel.
pub const POOLS_TAG: &str = "pools";

/// Index of the `draws.tsv` column `name` (one of [`DRAW_COLUMNS`]).
fn draw_column(name: &str) -> usize {
    DRAW_COLUMNS
        .iter()
        .position(|c| *c == name)
        .expect("draw column")
}

/// Whether a `draws.tsv` row is one of pass 9's calls or a pass 4 pool cel.
pub fn is_weather_row(row: &[String]) -> bool {
    let at = row[draw_column("at")].as_str();
    let addr = at
        .strip_prefix("0x")
        .and_then(|h| u32::from_str_radix(h, 16).ok());
    match row[draw_column("op")].as_str() {
        "DrawLine" | "DrawBox" => at == PASS9_TAG || addr.is_some_and(|a| PASS9_AT.contains(&a)),
        "CelDraw" => at == POOLS_TAG || addr.is_some_and(|a| POOLS_AT.contains(&a)),
        _ => false,
    }
}

impl FactSet {
    /// `--skip-weather`: drops the weather rows (pass 9 calls and pass 4 pool cels).
    pub fn without_weather(mut self) -> FactSet {
        self.draws.rows.retain(|r| !is_weather_row(r));
        self
    }
}

/// Reads both directories and compares them. The reference's `sprites.tsv` falls back to
/// `<original>/../../sprites.tsv`. `skip_weather` drops the weather rows from both sides first.
pub fn compare_dirs(
    original: &Path,
    d2rust: &Path,
    ignore: &[String],
    skip_weather: bool,
) -> Result<Outcome, FactsError> {
    let fallback = original.join("..").join("..");
    let mut o = FactSet::read(original, Some(&fallback))?;
    let mut d = FactSet::read(d2rust, None)?;
    if skip_weather {
        o = o.without_weather();
        d = d.without_weather();
    }
    Ok(compare(&o, &d, ignore))
}
