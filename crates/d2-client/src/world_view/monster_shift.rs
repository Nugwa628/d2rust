//! The monster palette shift `P` of the play preview: per monster, the
//! shift index and the map picks
//! from its class's `palshift.dat` or `RandTransforms.dat`, pushed into
//! the frame's map table once per class.
//!
//! Each class's block is the layout builds: the class at
//! `+0`, the file's 8 maps at `+4`, the second set at `+0x804` (never
//! read from the file, so zero).
//!
//! The play preview draws through `world_view::palshift` (the F5a path,
//! which also names the map `s<k>` for the facts export); this module
//! stays as the F5b implementation of the same rules, kept with its
//! tests until the two are folded into one.
//!
//! `utrans` 255 (necroskeleton, necromage) takes the owner relation the
//! 0xAC create read (`MonsterData::owner_relation`).

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_data::tables::{Monstats, Monstats2, Superuniques};

use crate::assets::path::FileSource;
use crate::bridge::world::{ClientWorld, KindData};
use crate::bridge::UnitKey;
use crate::rules::shading::{
    green_blood_switch, monster_shift_index, palshift_map, rand_transforms_map, shift_map,
    ShiftInputs, ShiftMap, ShiftMapInputs, MAP_BYTES, RAND_TRANSFORMS_MAPS,
};
use crate::scene::{MapId, MapTable};

/// Size of a class's palshift block.
const BLOCK_BYTES: usize = 0x1008;
/// The file's 8 maps.
const FILE_BYTES: usize = 8 * MAP_BYTES;

/// The palette shift table columns for one monster class.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassShift {
    /// `monstats` `Code` (lower case, trimmed): the `palshift.dat` folder.
    pub token: String,
    pub trans_lvl: u8,
    /// `monstats2` `noUniqueShift`.
    pub no_unique_shift: bool,
    /// `monstats2` `Utrans` by difficulty.
    pub utrans: [u8; 3],
    /// `monstats2` `localBlood`.
    pub local_blood: u8,
}

/// The shift columns of the user's tables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShiftRows {
    /// By `monstats` row.
    pub classes: BTreeMap<u32, ClassShift>,
    /// `superuniques` `Utrans` by difficulty, by row (hcIdx).
    pub superunique_utrans: Vec<[u8; 3]>,
}

impl ShiftRows {
    pub fn from_tables(
        monstats: &[Monstats],
        monstats2: &[Monstats2],
        supers: &[Superuniques],
    ) -> Self {
        let classes = monstats
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let ex = monstats2.get(usize::from(m.monstatsex));
                let token = m
                    .code
                    .iter()
                    .take_while(|&&b| b != 0 && b != b' ')
                    .map(|&b| (b as char).to_ascii_lowercase())
                    .collect();
                (
                    i as u32,
                    ClassShift {
                        token,
                        trans_lvl: m.translvl,
                        no_unique_shift: ex.is_some_and(|r| r.nouniqueshift),
                        utrans: ex.map_or([0; 3], |r| [r.utrans, r.utrans_n, r.utrans_h]),
                        local_blood: ex.map_or(0, |r| r.localblood),
                    },
                )
            })
            .collect();
        ShiftRows {
            classes,
            superunique_utrans: supers
                .iter()
                .map(|s| [s.utrans, s.utrans_n, s.utrans_h])
                .collect(),
        }
    }
}

/// The `monstats`, `monstats2` and `superuniques` rows of the user's tables.
pub fn load(archives: &dyn d2_data::bin::TableFiles) -> Result<ShiftRows, String> {
    use d2_data::tables::decode_all;
    let set = d2_data::bin::load_from(archives, "eng").map_err(|e| e.to_string())?;
    fn rows<T: d2_data::tables::Record>(
        set: &d2_data::bin::BinSet,
        name: &str,
    ) -> Result<Vec<T>, String> {
        match set.table(name) {
            Some(t) => decode_all(t).map_err(|e| e.to_string()),
            None => Ok(Vec::new()),
        }
    }
    Ok(ShiftRows::from_tables(
        &rows::<Monstats>(&set, "monstats")?,
        &rows::<Monstats2>(&set, "monstats2")?,
        &rows::<Superuniques>(&set, "superuniques")?,
    ))
}

/// The resident palette shift maps and each monster's `P`.
#[derive(Clone, Default)]
pub struct MonsterShifts {
    rows: Arc<ShiftRows>,
    source: Option<Arc<dyn FileSource>>,
    /// The class's block (`None`: no `palshift.dat`, "class without
    /// palshift data").
    blocks: BTreeMap<u32, Option<Arc<Vec<u8>>>>,
    /// `RandTransforms.dat` (`None`: not loaded).
    rand: Option<Option<Arc<Vec<u8>>>>,
    green_blood: Option<bool>,
    /// Pushed maps by (class, block offset) or (`u32::MAX`, RandTransforms map).
    pushed: BTreeMap<(u32, usize), MapId>,
    /// The map table length when `pushed` was filled (a new table drops them).
    table_len: usize,
    /// The frame's `P` by monster.
    units: BTreeMap<UnitKey, MapId>,
    error: Option<String>,
}

impl std::fmt::Debug for MonsterShifts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonsterShifts")
            .field("classes", &self.blocks.len())
            .field("units", &self.units)
            .finish()
    }
}

impl MonsterShifts {
    pub fn new(rows: ShiftRows, source: Option<Arc<dyn FileSource>>) -> Self {
        MonsterShifts {
            rows: Arc::new(rows),
            source,
            ..MonsterShifts::default()
        }
    }

    /// The frame's `P` by monster ([`Self::prepare`]).
    pub fn units(&self) -> &BTreeMap<UnitKey, MapId> {
        &self.units
    }

    /// The last failure (taken once).
    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

    fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.source.as_ref()?.read_file(name)?.ok()
    }

    fn block(&mut self, class: u32) -> Option<Arc<Vec<u8>>> {
        if let Some(b) = self.blocks.get(&class) {
            return b.clone();
        }
        let token = self
            .rows
            .classes
            .get(&class)
            .map(|c| c.token.clone())
            .unwrap_or_default();
        let block = (!token.is_empty())
            .then(|| {
                self.read(&format!(
                    "data\\global\\monsters\\{token}\\cof\\palshift.dat"
                ))
            })
            .flatten()
            .map(|file| {
                let mut b = vec![0u8; BLOCK_BYTES];
                b[..4].copy_from_slice(&class.to_le_bytes());
                let n = file.len().min(FILE_BYTES);
                b[4..4 + n].copy_from_slice(&file[..n]);
                Arc::new(b)
            });
        self.blocks.insert(class, block.clone());
        block
    }

    fn rand_transforms(&mut self) -> Option<Arc<Vec<u8>>> {
        if self.rand.is_none() {
            let file = self
                .read("data\\global\\monsters\\RandTransforms.dat")
                .filter(|f| f.len() >= MAP_BYTES * RAND_TRANSFORMS_MAPS as usize)
                .map(Arc::new);
            self.rand = Some(file);
        }
        self.rand.clone().flatten()
    }

    fn green_blood(&mut self) -> bool {
        if self.green_blood.is_none() {
            let txt = self.read("data\\local\\Color.txt");
            self.green_blood = Some(green_blood_switch(txt.as_deref()));
        }
        self.green_blood.unwrap_or(false)
    }

    /// The `P` of every monster in `world`, its maps pushed into `maps`
    /// once per (class, map).
    pub fn prepare(&mut self, world: &ClientWorld, maps: &mut MapTable) {
        if maps.len() < self.table_len {
            self.pushed.clear();
        }
        self.units.clear();
        let difficulty = usize::from(world.difficulty.min(2));
        let monsters: Vec<_> = world
            .units
            .values()
            .filter(|u| u.key.unit_type == 1)
            .filter_map(|u| match &u.kind {
                KindData::Monster(m) => Some((u.key, u.class, (**m).clone())),
                _ => None,
            })
            .collect();
        for (key, class, data) in monsters {
            match self.unit_map(class, &data, difficulty, maps) {
                Ok(Some(id)) => {
                    self.units.insert(key, id);
                }
                Ok(None) => {}
                Err(e) => self.error = Some(e),
            }
        }
        self.table_len = maps.len();
    }

    fn unit_map(
        &mut self,
        class: u32,
        data: &crate::bridge::world::MonsterData,
        difficulty: usize,
        maps: &mut MapTable,
    ) -> Result<Option<MapId>, String> {
        let rows = self.rows.clone();
        let Some(c) = rows.classes.get(&class) else {
            return Ok(None);
        };
        let superunique = data.flags & 0x2 != 0;
        let s = monster_shift_index(&ShiftInputs {
            class,
            // Only the low byte is kept.
            name_seed: data.name_seed & 0xFF,
            trans_lvl: c.trans_lvl,
            unique: data.flags & 0x8 != 0,
            no_unique_shift: c.no_unique_shift,
            utrans: c.utrans[difficulty],
            owner_relation: data.owner_relation,
            superunique_utrans: superunique.then(|| {
                rows.superunique_utrans
                    .get(usize::from(data.hc_idx))
                    .map_or(0, |u| u[difficulty])
            }),
        });
        let block = self.block(class);
        let green_blood = self.green_blood();
        let rand = self.rand_transforms();
        let choice = shift_map(
            s,
            &ShiftMapInputs {
                unit_type: 1,
                class,
                has_palshift: block.is_some(),
                green_blood,
                local_blood: u32::from(c.local_blood),
                rand_transforms_loaded: rand.is_some(),
            },
        )
        .map_err(|e| format!("{e:?}"))?;
        let (key, row) = match choice {
            ShiftMap::None => return Ok(None),
            ShiftMap::Palshift { offset } => {
                let block = block.expect("has_palshift");
                let row = *palshift_map(&block, offset).map_err(|e| format!("{e:?}"))?;
                ((class, offset), row)
            }
            ShiftMap::RandTransforms(m) => {
                let file = rand.expect("loaded");
                let row = *rand_transforms_map(&file, m).map_err(|e| format!("{e:?}"))?;
                ((u32::MAX, m as usize), row)
            }
        };
        Ok(Some(
            *self.pushed.entry(key).or_insert_with(|| maps.push(row)),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::path::MemorySource;
    use crate::bridge::world::ClientUnit;

    fn rows() -> ShiftRows {
        let mut classes = BTreeMap::new();
        classes.insert(
            19,
            ClassShift {
                token: "fa".into(),
                trans_lvl: 0,
                ..ClassShift::default()
            },
        );
        classes.insert(
            20,
            ClassShift {
                token: "zz".into(),
                ..ClassShift::default()
            },
        );
        ShiftRows {
            classes,
            superunique_utrans: Vec::new(),
        }
    }

    fn source() -> Arc<dyn FileSource> {
        let mut m = MemorySource::default();
        let mut file = vec![0u8; FILE_BYTES];
        for (k, b) in file.iter_mut().enumerate() {
            *b = (k / MAP_BYTES) as u8;
        }
        m.insert(r"DATA\GLOBAL\MONSTERS\FA\COF\palshift.dat", file);
        Arc::new(m)
    }

    fn monster(id: u32, class: u32) -> ClientUnit {
        let mut u = ClientUnit::new(UnitKey {
            unit_type: 1,
            guid: id,
        });
        u.class = class;
        u.kind = KindData::Monster(Box::default());
        u
    }

    // TransLvl 0 → s = 2 → palshift map 2.
    #[test]
    fn a_plain_monster_draws_with_palshift_map_2() {
        let mut world = ClientWorld::default();
        for u in [monster(1, 19), monster(2, 19), monster(3, 20)] {
            world.units.insert(u.key, u);
        }
        let mut shifts = MonsterShifts::new(rows(), Some(source()));
        let mut maps = MapTable::new();
        shifts.prepare(&world, &mut maps);
        let a = shifts.units()[&UnitKey {
            unit_type: 1,
            guid: 1,
        }];
        assert_eq!(
            shifts.units()[&UnitKey {
                unit_type: 1,
                guid: 2,
            }],
            a
        );
        // Class 20 has no palshift.dat: no P.
        assert!(!shifts.units().contains_key(&UnitKey {
            unit_type: 1,
            guid: 3,
        }));
        assert_eq!(maps.len(), 1);
        assert_eq!(maps.get(a).expect("pushed")[0], 2);
        assert!(shifts.take_error().is_none());
    }
}
