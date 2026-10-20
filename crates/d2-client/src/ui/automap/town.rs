//! Town art (DRLG callback +0x488): a grid of large cells for Lut Gholein, the
//! Pandemonium Fortress and Harrogath.

use super::cells::Cell;
use super::AutomapError;

/// The layer's town kind (+0x04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub enum TownKind {
    #[default]
    None = 0,
    /// Lut Gholein, `Act2Map`.
    LutGholein = 1,
    /// Pandemonium Fortress, `Act4Map`.
    Pandemonium = 2,
    /// Harrogath, `ExTnMap`.
    Harrogath = 3,
}

impl TownKind {
    pub fn from_u32(v: u32) -> Option<Self> {
        Some(match v {
            0 => TownKind::None,
            1 => TownKind::LutGholein,
            2 => TownKind::Pandemonium,
            3 => TownKind::Harrogath,
            _ => return None,
        })
    }
}

/// One row of the town table.
struct Grid {
    kind: TownKind,
    cols: i32,
    rows: i32,
    step: (i32, i32),
    first: (i32, i32),
}

/// Skipped cel ids of level 40.
pub const LUT_GHOLEIN_SKIPPED: [i32; 11] = [0, 10, 15, 16, 19, 20, 25, 30, 35, 36, 39];

/// The town kind and the cells of level `level` with picked file `f`
/// and centre tile (cx, cy), row-major. Any level other than 40, 103,
/// 109 is fatal 0x751.
pub fn town_cells(
    level: u32,
    f: u32,
    cx: i32,
    cy: i32,
) -> Result<(TownKind, Vec<Cell>), AutomapError> {
    let (x, y) = (((cx - cy) * 80) / 10, ((cx + cy) * 40) / 10);
    let (g, n0, skipped): (Grid, i32, &[i32]) = match level {
        40 => (
            Grid {
                kind: TownKind::LutGholein,
                cols: 5,
                rows: 4,
                step: (160, 100),
                first: (x - 453, y - 119),
            },
            if f == 1 { 0 } else { 20 },
            &LUT_GHOLEIN_SKIPPED,
        ),
        103 => (
            Grid {
                kind: TownKind::Pandemonium,
                cols: 2,
                rows: 2,
                step: (136, 90),
                first: (x - 133, y - 40),
            },
            0,
            &[],
        ),
        109 => (
            Grid {
                kind: TownKind::Harrogath,
                cols: 3,
                rows: 2,
                step: (180, 170),
                first: (x - 250, y - 15),
            },
            0,
            &[],
        ),
        _ => {
            return Err(AutomapError::fatal(
                0x751,
                "§6",
                format!("town art for level {level}"),
            ))
        }
    };
    let fit = |v: i32, code: u32, what: &str| {
        i16::try_from(v)
            .map_err(|_| AutomapError::fatal(code, "§6", format!("{what} {v} is not an i16")))
    };
    let mut out = Vec::new();
    let mut n = n0;
    for row in 0..g.rows {
        for col in 0..g.cols {
            if !skipped.contains(&n) {
                let cx = fit(g.first.0 + g.step.0 * col, 0x777, "x")?;
                let cy = fit(g.first.1 + g.step.1 * row, 0x778, "y")?;
                let cel = fit(n, 0x779, "cel")?;
                out.push(Cell::new(cel, cx, cy));
            }
            n += 1;
        }
    }
    Ok((g.kind, out))
}
