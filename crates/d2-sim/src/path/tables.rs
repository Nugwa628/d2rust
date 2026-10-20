//! The constant tables of `tables/sim/path-tables.tsv` (the table's
//! Constants lists the columns), embedded and parsed strictly (METHODS
//! M05, M07): every table has its exact row count, its exact set of
//! filled columns, contiguous indexes and entry addresses spaced by its
//! stride. Anything else is an error.

use super::PathError;

/// `tables/sim/path-tables.tsv`.
pub const PATH_TABLES_TSV: &str = include_str!("../../../../tables/sim/path-tables.tsv");

const HEADER: &str = "table\tindex\ta\tb\tc\td\te\tva";

/// (name, rows, filled columns a.., entry stride in bytes).
const SHAPES: [(&str, usize, usize, u32); 19] = [
    ("pathtype_flags", 18, 1, 4),
    ("pathtype_diroff", 18, 1, 4),
    ("pattern_of_size", 4, 1, 4),
    ("dir8_toward", 8, 2, 8),
    ("dir8_target", 8, 2, 8),
    ("testdir", 25, 3, 12),
    ("altdir", 25, 3, 3),
    ("dist8_path", 64, 1, 4),
    ("dist8_unit", 64, 1, 4),
    ("snap9", 81, 1, 4),
    ("tan", 128, 3, 12),
    ("tan_before", 127, 3, 12),
    ("dirdiff", 64, 1, 4),
    ("field_dx", 9, 1, 4),
    ("field_dy", 9, 1, 4),
    ("velmod_player", 20, 5, 20),
    ("velmod_monster", 16, 5, 20),
    ("velmod_monster_x", 16, 5, 20),
    ("animstat", 5, 3, 12),
];

/// Every table of `path-tables.tsv`, one entry per row, columns a.. in
/// order (only the filled ones).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathTables {
    /// Path type → flags.
    pub pathtype_flags: Vec<u32>,
    /// Path type → direction offset.
    pub pathtype_diroff: Vec<i32>,
    /// Size 0..3 → collision pattern.
    pub pattern_of_size: Vec<u32>,
    /// Direction → (dx, dy).
    pub dir8_toward: Vec<[i32; 2]>,
    pub dir8_target: Vec<[i32; 2]>,
    pub testdir: Vec<[i32; 3]>,
    /// 255 = none.
    pub altdir: Vec<[i32; 3]>,
    /// Index ax + 8·ay.
    pub dist8_path: Vec<i32>,
    pub dist8_unit: Vec<i32>,
    /// Index (dx + 4) + 9·(dy + 4).
    pub snap9: Vec<i32>,
    /// (x, y, angle).
    pub tan: Vec<[i32; 3]>,
    /// The 127 rows of 12 bytes before `tan`: row k is `tan[k − 127]`,
    /// read by a wrapped (negative) index.
    pub tan_before: Vec<[i32; 3]>,
    pub dirdiff: Vec<i32>,
    /// `ExpField.D2` direction byte → step.
    pub field_dx: Vec<i32>,
    pub field_dy: Vec<i32>,
    /// (by passive skill, velocity modifier, c, d, e).
    pub velmod_player: Vec<[i32; 5]>,
    pub velmod_monster: Vec<[i32; 5]>,
    pub velmod_monster_x: Vec<[i32; 5]>,
    /// (has base, base, stat).
    pub animstat: Vec<[i32; 3]>,
}

impl PathTables {
    /// The embedded tables.
    pub fn spec() -> Result<Self, PathError> {
        Self::from_tsv(PATH_TABLES_TSV)
    }

    /// Strict parse (module doc).
    pub fn from_tsv(tsv: &str) -> Result<Self, PathError> {
        let bad = |line: usize, why: String| PathError::Tsv(format!("line {line}: {why}"));
        let mut lines = tsv.lines().enumerate();
        match lines.next() {
            Some((_, h)) if h == HEADER => {}
            _ => return Err(bad(1, "header".into())),
        }
        // Rows per table, in SHAPES order.
        let mut rows: Vec<Vec<Vec<i32>>> = vec![Vec::new(); SHAPES.len()];
        let mut base: Vec<u32> = vec![0; SHAPES.len()];
        for (i, line) in lines {
            let n = i + 1;
            let cells: Vec<&str> = line.split('\t').collect();
            if cells.len() != 8 {
                return Err(bad(n, format!("{} cells", cells.len())));
            }
            let t = SHAPES
                .iter()
                .position(|s| s.0 == cells[0])
                .ok_or_else(|| bad(n, format!("unknown table {:?}", cells[0])))?;
            let (_, count, cols, stride) = SHAPES[t];
            let index: usize = cells[1]
                .parse()
                .map_err(|_| bad(n, format!("index {:?}", cells[1])))?;
            if index != rows[t].len() || index >= count {
                return Err(bad(n, format!("index {index} out of order")));
            }
            let mut vals = Vec::with_capacity(cols);
            for (c, cell) in cells[2..7].iter().enumerate() {
                if c < cols {
                    vals.push(
                        cell.parse::<i32>()
                            .map_err(|_| bad(n, format!("value {cell:?}")))?,
                    );
                } else if !cell.is_empty() {
                    return Err(bad(n, format!("column {} must be empty", c + 2)));
                }
            }
            let va = cells[7]
                .strip_prefix("0x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .ok_or_else(|| bad(n, format!("va {:?}", cells[7])))?;
            if index == 0 {
                base[t] = va;
            } else if Some(va) != base[t].checked_add(stride * index as u32) {
                return Err(bad(n, format!("va {va:#x} off the stride")));
            }
            rows[t].push(vals);
        }
        for (t, s) in SHAPES.iter().enumerate() {
            if rows[t].len() != s.1 {
                return Err(PathError::Tsv(format!(
                    "table {} has {} rows, want {}",
                    s.0,
                    rows[t].len(),
                    s.1
                )));
            }
        }
        let mut it = rows.into_iter();
        let mut next = || it.next().expect("one per shape");
        let one = |v: Vec<Vec<i32>>| v.into_iter().map(|r| r[0]).collect::<Vec<i32>>();
        let unsigned = |v: Vec<i32>, name: &str| {
            v.into_iter()
                .map(|x| u32::try_from(x).map_err(|_| PathError::Tsv(format!("{name}: {x} < 0"))))
                .collect::<Result<Vec<u32>, _>>()
        };
        fn arr<const N: usize>(v: Vec<Vec<i32>>) -> Vec<[i32; N]> {
            v.into_iter()
                .map(|r| r.try_into().expect("column count checked"))
                .collect()
        }
        Ok(Self {
            pathtype_flags: unsigned(one(next()), "pathtype_flags")?,
            pathtype_diroff: one(next()),
            pattern_of_size: unsigned(one(next()), "pattern_of_size")?,
            dir8_toward: arr(next()),
            dir8_target: arr(next()),
            testdir: arr(next()),
            altdir: arr(next()),
            dist8_path: one(next()),
            dist8_unit: one(next()),
            snap9: one(next()),
            tan: arr(next()),
            tan_before: arr(next()),
            dirdiff: one(next()),
            field_dx: one(next()),
            field_dy: one(next()),
            velmod_player: arr(next()),
            velmod_monster: arr(next()),
            velmod_monster_x: arr(next()),
            animstat: arr(next()),
        })
    }
}
