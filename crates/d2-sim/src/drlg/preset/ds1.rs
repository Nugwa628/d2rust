//! DS1 files as the preset code keeps them, the object → preset
//! unit conversion and the process-wide cache keyed by path.
//! The bytes are parsed outside the sim (`d2_formats::ds1`); the
//! sim receives a [`Ds1Input`] and applies the parser rules here.

use super::data::PresetData;
use super::{PathPoint, PresetError, PresetUnit};

/// Orientation remap of v < 7 files, a 42-entry table: the
/// 25 entries of the DS1 orientation lookup, then 17
/// more.
pub const ORIENTATION_REMAP: [u32; 42] = [
    0x00, 0x01, 0x02, 0x01, 0x02, 0x03, 0x03, 0x05, 0x05, 0x06, 0x06, 0x07, 0x07, 0x08, 0x09, 0x0A,
    0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x14, 0, 0, 1, 2, 5, 5, 7, 9, 11, 13, 14, 15,
    16, 17, 18, 19, 20,
];

/// Object type ids of the DS1 object records.
pub mod unit_type {
    pub const MONSTER: u32 = 1;
    pub const OBJECT: u32 = 2;
    pub const ITEM: u32 = 4;
}

/// One DS1 object record as stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ds1ObjectInput {
    pub kind: u32,
    pub id: u32,
    pub x: u32,
    pub y: u32,
    /// Stored flags (v ≥ 6; pass 0 otherwise).
    pub flags: u32,
}

/// One DS1 path record as stored: the unit position and its points
/// (x, y, action as stored; the action is ignored for v < 15).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ds1PathInput {
    pub x: u32,
    pub y: u32,
    pub points: Vec<(u32, u32, u32)>,
}

/// A parsed DS1 file as plain data: stored values,
/// before the parser rules apply.
///
/// Provider notes (`d2_formats::ds1::Ds1`): `width`/`height` here are the
/// **stored** values (d2_formats adds 1); layers are `(W+1)·(H+1)` cells;
/// `orientations` are the stored values (d2_formats already maps v < 7
/// files with the 25-entry table, so pass the raw cells for those);
/// `act` is the stored value read as i32 (0 for v < 8).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ds1Input {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub act: i32,
    pub tag_type: u32,
    pub walls: Vec<Vec<u32>>,
    pub orientations: Vec<Vec<u32>>,
    pub floors: Vec<Vec<u32>>,
    pub shadow: Vec<u32>,
    pub objects: Vec<Ds1ObjectInput>,
    pub paths: Vec<Ds1PathInput>,
}

/// Parsed DS1 files by path (no I/O in the sim). Paths are the lvlprest
/// `File1`–`File6` strings; the provider applies and
/// resolves them under `DATA\GLOBAL\TILES\`.
pub trait Ds1Source {
    fn ds1(&self, path: &[u8]) -> Option<&Ds1Input>;
}

/// The DS1 file record (0x5C bytes) after the parser rules.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ds1File {
    pub version: u32,
    /// Stored width and height (tiles; cells per row = width + 1).
    pub width: u32,
    pub height: u32,
    /// Act after the parser's act rule.
    pub act: i32,
    /// Tag type after the parser's tag rule.
    pub tag_type: u32,
    /// Wall layers and their orientation layers (after step 6).
    pub walls: Vec<Vec<u32>>,
    pub orientations: Vec<Vec<u32>>,
    /// Floor layers (none for v < 4, step 5).
    pub floors: Vec<Vec<u32>>,
    pub shadow: Vec<u32>,
    /// The file's preset-unit list, head first (reverse file order).
    pub units: Vec<PresetUnit>,
}

impl Ds1File {
    /// Cells per row (`width + 1`).
    pub fn stride(&self) -> usize {
        self.width as usize + 1
    }

    /// The parser on a parsed file.
    pub fn from_input(input: &Ds1Input, pd: &PresetData) -> Result<Self, PresetError> {
        let v = input.version;
        let cells = (input.width as usize + 1) * (input.height as usize + 1);
        let bad = |what: &str| PresetError::BadDs1(what.to_string());
        if input.walls.len() != input.orientations.len() {
            return Err(bad("wall and orientation layer counts differ"));
        }
        let all = input
            .walls
            .iter()
            .chain(&input.orientations)
            .chain(&input.floors)
            .chain(std::iter::once(&input.shadow));
        for l in all {
            if l.len() != cells {
                return Err(bad("layer size is not (W+1)·(H+1)"));
            }
        }
        // Step 2: act.
        let act = if v < 8 { 0 } else { input.act.min(4) };
        // Step 3.
        let tag_type = if v < 10 { 0 } else { input.tag_type };
        // Step 6: v < 7 orientation remap.
        let mut orientations = input.orientations.clone();
        if v < 7 {
            for layer in &mut orientations {
                for o in layer.iter_mut() {
                    // TODO: the table has no range check
                    // beyond the 42 entries; what it reads past them is
                    // not stated.
                    *o = *ORIENTATION_REMAP
                        .get(*o as usize)
                        .ok_or_else(|| bad("v < 7 orientation beyond the 42-entry table"))?;
                }
            }
        }
        let mut file = Ds1File {
            version: v,
            width: input.width,
            height: input.height,
            act,
            tag_type,
            walls: input.walls.clone(),
            orientations,
            // Step 5: v < 4 keeps floor count 0.
            floors: if v < 4 {
                Vec::new()
            } else {
                input.floors.clone()
            },
            shadow: input.shadow.clone(),
            units: Vec::new(),
        };
        // Step 8: objects (v ≥ 2).
        if v >= 2 {
            for o in &input.objects {
                if let Some(u) = convert_object(o, v, act, pd)? {
                    file.units.insert(0, u);
                }
            }
        }
        // Step 10: paths (v ≥ 14).
        if v >= 14 {
            for p in &input.paths {
                if p.points.is_empty() {
                    continue;
                }
                let (x, y) = (p.x as i32, p.y as i32);
                if let Some(u) = file.units.iter_mut().find(|u| u.x == x && u.y == y) {
                    u.path = Some(
                        p.points
                            .iter()
                            .map(|&(px, py, a)| PathPoint {
                                action: if v >= 15 { a } else { 1 },
                                x: px as i32,
                                y: py as i32,
                            })
                            .collect(),
                    );
                }
            }
        }
        Ok(file)
    }
}

/// One DS1 object record → a file preset unit, or `None` if
/// dropped.
fn convert_object(
    o: &Ds1ObjectInput,
    v: u32,
    act: i32,
    pd: &PresetData,
) -> Result<Option<PresetUnit>, PresetError> {
    let m = pd.monstats_count as i32;
    let s = pd.superuniques_count as i32;
    let mut unit_type = o.kind;
    let id = o.id as i32;
    let (mut mode, mut class) = match o.kind {
        unit_type::MONSTER => {
            if v <= 4 {
                return Ok(None);
            }
            (1, monster_class(id, act, m, s, pd)?)
        }
        unit_type::OBJECT => {
            let c = if v >= 6 {
                if (o.id as usize) < super::data::OBJPRESET_IDS {
                    let a = usize::try_from(act).map_err(|_| PresetError::NegativeAct(act))?;
                    pd.tables.objpreset[a][o.id as usize]
                } else {
                    id - 150
                }
            } else if o.id == 573 {
                -1
            } else {
                id
            };
            (0, c)
        }
        unit_type::ITEM => {
            let c = if v >= 5 {
                if o.id != 0 {
                    return Err(PresetError::ItemCodeBeyondTable(o.id));
                }
                pd.hdm_item
            } else {
                id
            };
            (3, c)
        }
        _ => (0, id),
    };
    // Monster → object conversions for ids in [0, M).
    if o.kind == unit_type::MONSTER && (0..m).contains(&class) {
        let obj = match (act, class) {
            (2, 297) => Some(382),
            (2, 366) => Some(404),
            (4, 514) => Some(461),
            (4, 537..=539) => Some(1013 - class),
            _ => None,
        };
        if let Some(c) = obj {
            unit_type = unit_type::OBJECT;
            mode = 0;
            class = c;
        }
    }
    if class < 0 {
        return Ok(None);
    }
    Ok(Some(PresetUnit {
        unit_type,
        class,
        mode,
        x: o.x as i32,
        y: o.y as i32,
        flags: o.flags,
        path: None,
    }))
}

/// Monster id through the act's monpreset range.
fn monster_class(id: i32, act: i32, m: i32, s: i32, pd: &PresetData) -> Result<i32, PresetError> {
    let Ok(a) = usize::try_from(act) else {
        // TODO: a negative DS1 act (kept by the parser's act rule)
        // indexes the monpreset ranges out of bounds; not stated.
        return Err(PresetError::NegativeAct(act));
    };
    let (Some(first), count) = (pd.monpreset_acts.first[a], pd.monpreset_acts.count[a]) else {
        return Ok(id);
    };
    if id < 0 || id as u32 >= count {
        return Ok(id);
    }
    let row = pd
        .monpreset
        .get((first + id as u32) as usize)
        .ok_or_else(|| PresetError::BadData("monpreset row beyond the table".into()))?;
    let p = i32::from(row.place);
    Ok(match row.kind {
        0 => p + s + m,
        1 => p,
        2 => p + m,
        _ => -1,
    })
}

/// One cache entry: path, reference count, the shared record.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CacheEntry {
    path: Vec<u8>,
    refs: i32,
    file: Ds1File,
}

/// A cached DS1 (stable while its entry lives).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ds1Id(pub u32);

/// The process-wide DS1 cache, keyed by the
/// exact path bytes. The record is shared: the preset grid init ORs
/// edge bits into its layers, and every map using the file sees them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ds1Cache {
    entries: Vec<Option<CacheEntry>>,
}

impl Ds1Cache {
    /// Load: hit → refs + 1; miss → parse, refs 1.
    pub fn load(
        &mut self,
        path: &[u8],
        src: &dyn Ds1Source,
        pd: &PresetData,
    ) -> Result<Ds1Id, PresetError> {
        if let Some(i) = self
            .entries
            .iter()
            .position(|e| e.as_ref().is_some_and(|e| e.path == path))
        {
            if let Some(e) = self.entries[i].as_mut() {
                e.refs += 1;
            }
            return Ok(Ds1Id(i as u32));
        }
        let input = src
            .ds1(path)
            .ok_or_else(|| PresetError::MissingDs1(String::from_utf8_lossy(path).into_owned()))?;
        let entry = CacheEntry {
            path: path.to_vec(),
            refs: 1,
            file: Ds1File::from_input(input, pd)?,
        };
        let i = match self.entries.iter().position(Option::is_none) {
            Some(i) => {
                self.entries[i] = Some(entry);
                i
            }
            None => {
                self.entries.push(Some(entry));
                self.entries.len() - 1
            }
        };
        Ok(Ds1Id(i as u32))
    }

    /// Release: refs − 1; at ≤ 0 the entry is freed.
    pub fn release(&mut self, id: Ds1Id) {
        let slot = &mut self.entries[id.0 as usize];
        if let Some(e) = slot.as_mut() {
            e.refs -= 1;
            if e.refs <= 0 {
                *slot = None;
            }
        }
    }

    pub fn file(&self, id: Ds1Id) -> &Ds1File {
        &self.entries[id.0 as usize].as_ref().expect("live DS1").file
    }

    pub fn file_mut(&mut self, id: Ds1Id) -> &mut Ds1File {
        &mut self.entries[id.0 as usize].as_mut().expect("live DS1").file
    }

    /// Reference count of a path (0 if not cached).
    pub fn refs(&self, path: &[u8]) -> i32 {
        self.entries
            .iter()
            .flatten()
            .find(|e| e.path == path)
            .map_or(0, |e| e.refs)
    }
}
