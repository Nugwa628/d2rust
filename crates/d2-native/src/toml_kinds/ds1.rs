//! Map pieces (`.ds1`) as `P.toml`.
//!
//! A DS1 is one hand-built piece of a level: a grid of cells in several
//! layers (walls, each with an orientation layer, floors, one shadow
//! layer and an optional tag layer), the tile sets it draws from, the
//! objects and monsters placed on it, groups of cells, and the patrol
//! paths of its monsters.
//!
//! In the sidecar every grid layer is a `[[layer]]` table whose `rows` are
//! strings of space-separated cells, each cell 8 lowercase hex digits (the
//! packed tile reference). Layers appear in file order: each wall followed
//! by its orientation, then the floors, the shadow, the tag layer.
//! `width` and `height` are the decoded struct's (the stored header values
//! plus one).

use d2_formats::ds1::{Ds1, Ds1Group, Ds1Object, Ds1Path, Ds1PathPoint};

use super::text::{
    check_round_trip, hex_bytes, latin1, not_representable, parse_bare_hex, toml_strs, unhex_bytes,
    unlatin1, Out, Tab, TextError,
};

/// Most rows a grid may have when its width is 0. A header can claim 2^32
/// empty rows, and the sidecar would be gigabytes of empty strings.
const MAX_EMPTY_ROWS: u32 = 1 << 20;

/// Writes a DS1 sidecar. A layer must have `height` rows of `width` cells,
/// and there must be one orientation layer per wall layer.
pub fn write_ds1(file: &str, d: &Ds1) -> Result<String, TextError> {
    let cells = u64::from(d.width) * u64::from(d.height);
    let mut grids = d
        .walls
        .iter()
        .chain(&d.orientations)
        .chain(&d.floors)
        .chain(std::iter::once(&d.shadow))
        .chain(d.tags.iter());
    if grids.any(|l| l.len() as u64 != cells) {
        return Err(not_representable(
            file,
            "a layer is not width × height cells",
        ));
    }
    if d.orientations.len() != d.walls.len() {
        return Err(not_representable(
            file,
            "orientation layers differ from wall layers",
        ));
    }
    if d.width == 0 && d.height > MAX_EMPTY_ROWS {
        return Err(not_representable(file, "empty grid with too many rows"));
    }
    let mut o = Out::header("ds1");
    o.int("version", d.version);
    o.int("width", d.width);
    o.int("height", d.height);
    o.int("act", d.act);
    o.int("tag_type", d.tag_type);
    if let Some(u) = d.unknown_header {
        o.hex("unknown_header", u64::from_be_bytes(u), 16);
    }
    let names: Vec<String> = d.files.iter().map(|f| latin1(f)).collect();
    o.strs("files", names.iter().map(String::as_str));
    if let Some(u) = d.unknown_groups {
        o.hex("unknown_groups", u64::from(u32::from_be_bytes(u)), 8);
    }
    o.boolean("groups_truncated", d.groups_truncated);
    o.str("trailing", &hex_bytes(&d.trailing));

    let emit = |o: &mut Out, kind: &str, index: usize, g: &[u32]| {
        o.table("[[layer]]");
        o.str("kind", kind);
        o.int("index", index);
        o.raw("rows", grid_rows(g, d.width, d.height));
    };
    for (i, (wall, orientation)) in d.walls.iter().zip(&d.orientations).enumerate() {
        emit(&mut o, "wall", i, wall);
        emit(&mut o, "orientation", i, orientation);
    }
    for (i, g) in d.floors.iter().enumerate() {
        emit(&mut o, "floor", i, g);
    }
    emit(&mut o, "shadow", 0, &d.shadow);
    if let Some(g) = &d.tags {
        emit(&mut o, "tag", 0, g);
    }
    for ob in &d.objects {
        o.table("[[object]]");
        o.int("kind", ob.kind);
        o.int("id", ob.id);
        o.int("x", ob.x);
        o.int("y", ob.y);
        o.hex("flags", u64::from(ob.flags), 8);
    }
    for g in &d.groups {
        o.table("[[group]]");
        o.int("x", g.x);
        o.int("y", g.y);
        o.int("width", g.width);
        o.int("height", g.height);
        o.int("unknown", g.unknown);
    }
    for p in &d.paths {
        o.table("[[path]]");
        o.int("x", p.x);
        o.int("y", p.y);
        for pt in &p.points {
            o.table("[[path.point]]");
            o.int("x", pt.x);
            o.int("y", pt.y);
            o.int("action", pt.action);
        }
    }
    Ok(o.0)
}

/// A grid as a TOML array of row strings.
fn grid_rows(g: &[u32], width: u32, height: u32) -> String {
    let w = width as usize;
    toml_strs((0..height as usize).map(|y| {
        let cells: Vec<String> = g[y * w..(y + 1) * w]
            .iter()
            .map(|c| format!("{c:08x}"))
            .collect();
        cells.join(" ")
    }))
}

/// The `rows` of one `[[layer]]` table as a flat cell list.
fn read_grid(t: &mut Tab, width: u32, height: u32) -> Result<Vec<u32>, TextError> {
    let rows = t.str_array("rows")?;
    if rows.len() != height as usize {
        return Err(t.err(format!("{} rows, expected {height}", rows.len())));
    }
    let mut cells = Vec::with_capacity(width as usize * height as usize);
    for (y, row) in rows.iter().enumerate() {
        let before = cells.len();
        if width > 0 {
            for c in row.split(' ') {
                let v = parse_bare_hex(c, 8).ok_or_else(|| {
                    t.err(format!("row {y}: cell {c:?} is not 8 lowercase hex digits"))
                })?;
                cells.push(v as u32);
            }
        } else if !row.is_empty() {
            return Err(t.err(format!("row {y} has cells in a width-0 grid")));
        }
        if cells.len() - before != width as usize {
            return Err(t.err(format!(
                "row {y} has {} cells, expected {width}",
                cells.len() - before
            )));
        }
    }
    Ok(cells)
}

/// The grid layers of a map piece, gathered from the `[[layer]]` tables.
#[derive(Default)]
struct Grids {
    walls: Vec<Vec<u32>>,
    orientations: Vec<Vec<u32>>,
    floors: Vec<Vec<u32>>,
    shadow: Option<Vec<u32>>,
    tags: Option<Vec<u32>>,
}

impl Grids {
    /// Files one `[[layer]]` table. Wall, orientation and floor layers are
    /// numbered from 0 in order; the shadow and tag layers appear at most
    /// once, with index 0.
    fn add(&mut self, mut l: Tab, width: u32, height: u32) -> Result<(), TextError> {
        let kind = l.string("kind")?;
        let index = l.int("index", 0, i64::MAX)? as usize;
        let grid = read_grid(&mut l, width, height)?;
        let list = match kind.as_str() {
            "wall" => &mut self.walls,
            "orientation" => &mut self.orientations,
            "floor" => &mut self.floors,
            "shadow" | "tag" => {
                let single = if kind == "shadow" {
                    &mut self.shadow
                } else {
                    &mut self.tags
                };
                if index != 0 || single.is_some() {
                    return Err(l.err(format!("{kind} layer repeated or indexed")));
                }
                *single = Some(grid);
                return l.finish();
            }
            other => return Err(l.err(format!("unknown layer kind {other:?}"))),
        };
        if index != list.len() {
            return Err(l.err(format!(
                "{kind} layer index {index}, expected {}",
                list.len()
            )));
        }
        list.push(grid);
        l.finish()
    }
}

/// Reads a DS1 sidecar back into the decoded struct.
pub fn read_ds1(file: &str, text: &str) -> Result<Ds1, TextError> {
    let mut t = Tab::parse(file, text, "ds1")?;
    let version = t.u32("version")?;
    let width = t.u32("width")?;
    let height = t.u32("height")?;
    let act = t.u32("act")?;
    let tag_type = t.u32("tag_type")?;
    let unknown_header = if t.has("unknown_header") {
        Some(t.hex("unknown_header", 16)?.to_be_bytes())
    } else {
        None
    };
    let mut files = Vec::new();
    for name in t.str_array("files")? {
        files.push(unlatin1(file, "files", &name)?);
    }
    let unknown_groups = if t.has("unknown_groups") {
        Some((t.hex("unknown_groups", 8)? as u32).to_be_bytes())
    } else {
        None
    };
    let groups_truncated = t.boolean("groups_truncated")?;
    let trailing = t.string("trailing")?;
    let trailing =
        unhex_bytes(&trailing).ok_or_else(|| t.err("`trailing` is not lowercase hex bytes"))?;

    if width == 0 && height > MAX_EMPTY_ROWS {
        return Err(t.err("empty grid with too many rows"));
    }
    let mut grids = Grids::default();
    for l in t.tables("layer")? {
        grids.add(l, width, height)?;
    }
    if grids.walls.len() != grids.orientations.len() {
        return Err(t.err("wall and orientation layer counts differ"));
    }
    let shadow = grids.shadow.ok_or_else(|| t.err("no shadow layer"))?;

    let mut objects = Vec::new();
    for mut o in t.tables("object")? {
        objects.push(Ds1Object {
            kind: o.u32("kind")?,
            id: o.u32("id")?,
            x: o.u32("x")?,
            y: o.u32("y")?,
            flags: o.hex("flags", 8)? as u32,
        });
        o.finish()?;
    }
    let mut groups = Vec::new();
    for mut g in t.tables("group")? {
        groups.push(Ds1Group {
            x: g.u32("x")?,
            y: g.u32("y")?,
            width: g.u32("width")?,
            height: g.u32("height")?,
            unknown: g.u32("unknown")?,
        });
        g.finish()?;
    }
    let mut paths = Vec::new();
    for mut p in t.tables("path")? {
        let x = p.u32("x")?;
        let y = p.u32("y")?;
        let mut points = Vec::new();
        for mut pt in p.tables("point")? {
            points.push(Ds1PathPoint {
                x: pt.u32("x")?,
                y: pt.u32("y")?,
                action: pt.u32("action")?,
            });
            pt.finish()?;
        }
        p.finish()?;
        paths.push(Ds1Path { x, y, points });
    }
    t.finish()?;
    Ok(Ds1 {
        version,
        width,
        height,
        act,
        tag_type,
        files,
        unknown_header,
        walls: grids.walls,
        orientations: grids.orientations,
        floors: grids.floors,
        shadow,
        tags: grids.tags,
        objects,
        unknown_groups,
        groups,
        groups_truncated,
        paths,
        trailing,
    })
}

/// Round-trip check for DS1: write, read back, compare.
pub fn check_ds1(file: &str, original: &Ds1) -> Result<(), TextError> {
    check_round_trip(file, original, write_ds1, read_ds1)
}
