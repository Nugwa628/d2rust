//! The monster palette shift of the preview: the shift index `s` of each
//! monster in view (steps 1–5) and the map it selects, from the
//! class's `palshift.dat` or `RandTransforms.dat`, pushed into the
//! frame's map table once per map.
//!
//! The green-blood switch is read from `Color.txt` (off when
//! absent); `Utrans` 255 needs the owner relation of Open question 7, so such a
//! monster's map is reported unknown instead of guessed.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use d2_data::tables::{Monstats, Monstats2, Superuniques};

use crate::assets::path::FileSource;
use crate::bridge::world::{ClientUnit, KindData};
use crate::rules::shading::{
    green_blood_switch, monster_shift_index, palshift_map, rand_transforms_map, shift_map,
    ShiftInputs, ShiftMap, ShiftMapInputs,
};
use crate::rules::unit_composite::component::{S7, S8};
use crate::scene::{CelPal, MapId, MapTable};

/// Path of the shared shift maps.
pub const RAND_TRANSFORMS: &str = "data\\global\\monsters\\RandTransforms.dat";

/// The green-blood switch file.
pub const COLOR_TXT: &str = "data\\local\\Color.txt";

/// A monstats `Code` as a path token: it ends at its first zero or
/// space (the COF paths' rule, `unit_composite`).
fn token_of(code: &[u8; 4]) -> String {
    let n = code.iter().position(|&b| b == 0 || b == b' ').unwrap_or(4);
    String::from_utf8_lossy(&code[..n]).into_owned()
}

/// The class's `palshift.dat`: `<token>\cof\palshift.dat`.
pub fn palshift_path(token: &str) -> String {
    format!("data\\global\\monsters\\{token}\\cof\\palshift.dat")
}

/// The shift columns of one `monstats` row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShiftRow {
    /// `monstats` `Code` (the `palshift.dat` folder).
    pub token: String,
    pub trans_lvl: u8,
    /// `monstats2` `Utrans` by difficulty.
    pub utrans: [u8; 3],
    pub no_unique_shift: bool,
    pub local_blood: u8,
}

/// The table columns the shift reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShiftRows {
    /// By monster class.
    pub monsters: Vec<ShiftRow>,
    /// `superuniques` `Utrans` by difficulty, by hcIdx.
    pub superuniques: Vec<[u8; 3]>,
}

impl ShiftRows {
    /// The rows from the user's `.bin` tables (`data\global\excel`).
    pub fn live(source: &dyn FileSource) -> Result<Self, String> {
        use super::unit_assets::read_table;
        Ok(Self::from_tables(
            &read_table::<Monstats>(source)?,
            &read_table::<Monstats2>(source)?,
            &read_table::<Superuniques>(source)?,
        ))
    }

    pub fn from_tables(m: &[Monstats], m2: &[Monstats2], su: &[Superuniques]) -> Self {
        let monsters = m
            .iter()
            .map(|r| {
                let x = m2.get(usize::from(r.monstatsex));
                ShiftRow {
                    token: token_of(&r.code),
                    trans_lvl: r.translvl,
                    utrans: x.map_or([0; 3], |x| [x.utrans, x.utrans_n, x.utrans_h]),
                    no_unique_shift: x.is_some_and(|x| x.nouniqueshift),
                    local_blood: x.map_or(0, |x| x.localblood),
                }
            })
            .collect();
        let superuniques = su
            .iter()
            .map(|r| [r.utrans, r.utrans_n, r.utrans_h])
            .collect();
        ShiftRows {
            monsters,
            superuniques,
        }
    }
}

/// What the look of one monster component names: its remap and the
/// palette argument the facts export reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShiftLook {
    pub remap: Option<MapId>,
    pub pal: CelPal,
}

#[derive(Debug, Default)]
struct Loaded {
    /// The 2048 bytes of each token's `palshift.dat`; `None`: no file.
    files: BTreeMap<String, Option<Arc<Vec<u8>>>>,
    /// `RandTransforms.dat`, once read (`None` inside: absent).
    rand: Option<Option<Arc<Vec<u8>>>>,
    /// The green-blood switch, once read.
    green_blood: Option<bool>,
    /// Pushed maps: (token, `ShiftMap`) → map id.
    pushed: BTreeMap<(String, ShiftMap), MapId>,
    /// The classes whose block is loaded, as of the last `ensure`
    /// (`ClientWorld::palshift_classes`, class `palshift` data).
    blocks: BTreeSet<u32>,
}

/// The palette shifts of the monsters in view.
#[derive(Clone)]
pub struct PalShifts {
    rows: Arc<ShiftRows>,
    source: Option<Arc<dyn FileSource>>,
    difficulty: u8,
    loaded: Arc<Mutex<Loaded>>,
}

impl std::fmt::Debug for PalShifts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PalShifts")
            .field("classes", &self.rows.monsters.len())
            .field("difficulty", &self.difficulty)
            .finish()
    }
}

/// The shift step of one monster: its `s` and map, `Err` when the map
/// choice fails.
fn choose(
    rows: &ShiftRows,
    difficulty: u8,
    unit: &ClientUnit,
    has_palshift: bool,
    rand_loaded: bool,
    green_blood: bool,
) -> Option<Result<(u32, ShiftMap), ()>> {
    let KindData::Monster(m) = &unit.kind else {
        return None;
    };
    let row = rows.monsters.get(unit.class as usize)?;
    let d = usize::from(difficulty.min(2));
    let utrans = row.utrans[d];
    let superunique_utrans = (m.flags & 2 != 0).then(|| {
        rows.superuniques
            .get(usize::from(m.hc_idx))
            .map_or(0, |u| u[d])
    });
    let s = monster_shift_index(&ShiftInputs {
        class: unit.class,
        name_seed: m.name_seed,
        trans_lvl: row.trans_lvl,
        unique: m.flags & 8 != 0,
        no_unique_shift: row.no_unique_shift,
        utrans,
        owner_relation: m.owner_relation,
        superunique_utrans,
    });
    let map = shift_map(
        s,
        &ShiftMapInputs {
            unit_type: 1,
            class: unit.class,
            has_palshift,
            green_blood,
            local_blood: u32::from(row.local_blood),
            rand_transforms_loaded: rand_loaded,
        },
    )
    .map_err(|_| ())
    .map(|p| (s, p));
    Some(map)
}

impl PalShifts {
    pub fn new(rows: Arc<ShiftRows>, source: Option<Arc<dyn FileSource>>) -> Self {
        PalShifts {
            rows,
            source,
            difficulty: 0,
            loaded: Arc::default(),
        }
    }

    fn read(&self, path: &str) -> Option<Arc<Vec<u8>>> {
        match self.source.as_ref()?.read_file(path)? {
            Ok(b) => Some(Arc::new(b)),
            Err(_) => None,
        }
    }

    /// Reads the files and pushes the maps of the monsters of `units`
    /// (once per frame, before the build). `blocks`: the classes an S→C
    /// 0xAC loaded the `palshift` block of; a class outside it
    /// has no `palshift` data, whatever the archives hold.
    pub fn ensure(
        &mut self,
        units: &[&ClientUnit],
        blocks: &BTreeSet<u32>,
        difficulty: u8,
        maps: &mut MapTable,
    ) {
        self.difficulty = difficulty;
        let Ok(mut l) = self.loaded.lock() else {
            return;
        };
        if l.blocks != *blocks {
            l.blocks = blocks.clone();
        }
        if l.rand.is_none() {
            l.rand = Some(self.read(RAND_TRANSFORMS));
        }
        let rand = l.rand.clone().flatten();
        if l.green_blood.is_none() {
            let txt = self.read(COLOR_TXT);
            l.green_blood = Some(green_blood_switch(txt.as_deref().map(|v| v.as_slice())));
        }
        let green_blood = l.green_blood.unwrap_or(false);
        for unit in units.iter().filter(|u| u.key.unit_type == 1) {
            let Some(row) = self.rows.monsters.get(unit.class as usize) else {
                continue;
            };
            if !l.files.contains_key(&row.token) {
                let f = self.read(&palshift_path(&row.token));
                l.files.insert(row.token.clone(), f);
            }
            let file = l.files.get(&row.token).cloned().flatten();
            let has = file.is_some() && l.blocks.contains(&unit.class);
            let Some(Ok((_, map))) = choose(
                &self.rows,
                difficulty,
                unit,
                has,
                rand.is_some(),
                green_blood,
            ) else {
                continue;
            };
            let key = (row.token.clone(), map);
            if l.pushed.contains_key(&key) {
                continue;
            }
            let bytes = match (map, &file, &rand) {
                (ShiftMap::Palshift { offset }, Some(f), _) => {
                    // The class block's maps start 4 bytes in.
                    let mut block = vec![0u8; 4];
                    block.extend_from_slice(f);
                    palshift_map(&block, offset).ok().copied()
                }
                (ShiftMap::RandTransforms(k), _, Some(r)) => {
                    rand_transforms_map(r, k).ok().copied()
                }
                _ => None,
            };
            if let Some(b) = bytes {
                let id = maps.push(b);
                l.pushed.insert(key, id);
            }
        }
    }

    /// The shift look of component `component` of `unit` with palette
    /// index `palette_index` (a nonzero index takes the shift row
    /// instead); `None`: no shift applies.
    pub fn look(&self, unit: &ClientUnit, component: u8, palette_index: u8) -> Option<ShiftLook> {
        if unit.key.unit_type != 1 || palette_index != 0 || component == S7 || component == S8 {
            return None;
        }
        let unknown = ShiftLook {
            remap: None,
            pal: CelPal::Other,
        };
        let row = self.rows.monsters.get(unit.class as usize)?;
        let l = self.loaded.lock().ok()?;
        let file = l.files.get(&row.token)?;
        let rand = l.rand.clone().flatten();
        let has = file.is_some() && l.blocks.contains(&unit.class);
        let (s, map) = match choose(
            &self.rows,
            self.difficulty,
            unit,
            has,
            rand.is_some(),
            l.green_blood.unwrap_or(false),
        )? {
            Ok(x) => x,
            Err(()) => return Some(unknown),
        };
        let pal = match map {
            ShiftMap::None => return None,
            ShiftMap::Palshift { .. } => CelPal::Shift(s as u8),
            ShiftMap::RandTransforms(k) => CelPal::Shift(8 + k as u8),
        };
        match l.pushed.get(&(row.token.clone(), map)) {
            Some(&id) => Some(ShiftLook {
                remap: Some(id),
                pal,
            }),
            None => Some(unknown),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> ShiftRows {
        let mut monsters = vec![ShiftRow::default(); 20];
        monsters[19] = ShiftRow {
            token: "FA".into(),
            ..ShiftRow::default()
        };
        monsters[5] = ShiftRow {
            token: "XX".into(),
            utrans: [0, 255, 0],
            ..ShiftRow::default()
        };
        ShiftRows {
            monsters,
            superuniques: vec![[0, 0, 0], [3, 4, 5]],
        }
    }

    fn monster(class: u32, flags: u8, hc_idx: u16, name_seed: u16) -> ClientUnit {
        let mut u = ClientUnit::new(crate::bridge::world::UnitKey::new(1, 7));
        u.class = class;
        u.kind = KindData::Monster(Box::new(crate::bridge::world::MonsterData {
            name_seed,
            flags,
            hc_idx,
            ..Default::default()
        }));
        u
    }

    #[test]
    fn token_stops_at_a_space_or_zero() {
        assert_eq!(token_of(b"FA  "), "FA");
        assert_eq!(token_of(b"B9\0\0"), "B9");
        assert_eq!(
            palshift_path(&token_of(b"FA  ")),
            "data\\global\\monsters\\FA\\cof\\palshift.dat"
        );
    }

    #[test]
    fn trans_lvl_zero_is_map_two() {
        let r = rows();
        let u = monster(19, 0, 0, 0);
        let got = choose(&r, 0, &u, true, true, false).unwrap().unwrap();
        assert_eq!(got, (2, ShiftMap::Palshift { offset: 4 + 512 }));
        assert_eq!(
            choose(&r, 0, &u, false, true, false).unwrap().unwrap().1,
            ShiftMap::None
        );
    }

    #[test]
    fn superunique_utrans_by_difficulty() {
        let r = rows();
        let u = monster(19, 2, 1, 0);
        assert_eq!(choose(&r, 1, &u, true, true, false).unwrap().unwrap().0, 4);
        assert_eq!(choose(&r, 2, &u, true, true, false).unwrap().unwrap().0, 5);
    }

    #[test]
    fn utrans_255_reads_the_owner_relation() {
        let r = rows();
        let mut u = monster(5, 0, 0, 0);
        // The local player's own pet: relation 0, s = 1.
        let (s, _) = choose(&r, 1, &u, true, true, false).unwrap().unwrap();
        assert_eq!(s, 1);
        // No pet record: relation 1, s = 0.
        if let KindData::Monster(m) = &mut u.kind {
            m.owner_relation = true;
        }
        let (s, _) = choose(&r, 1, &u, true, true, false).unwrap().unwrap();
        assert_eq!(s, 0);
        // Normal difficulty: Utrans 0, TransLvl 0 gives 2.
        let (s, _) = choose(&r, 0, &u, true, true, false).unwrap().unwrap();
        assert_eq!(s, 2);
    }

    #[test]
    fn unique_takes_rand_transforms() {
        // test vector: class 5, seed 0x1234 → s 20 → map 12.
        let mut r = rows();
        r.monsters[5].utrans = [0; 3];
        let u = monster(5, 8, 0, 0x1234);
        let got = choose(&r, 0, &u, true, true, false).unwrap().unwrap();
        assert_eq!(got, (20, ShiftMap::RandTransforms(12)));
    }

    #[test]
    fn look_names_the_pushed_map() {
        struct Files;
        impl FileSource for Files {
            fn read_file(&self, name: &str) -> Option<Result<Vec<u8>, String>> {
                if name == palshift_path("FA") {
                    Some(Ok((0..2048).map(|i| (i / 256) as u8).collect()))
                } else {
                    None
                }
            }
        }
        let mut p = PalShifts::new(Arc::new(rows()), Some(Arc::new(Files)));
        let u = monster(19, 0, 0, 0);
        let mut maps = MapTable::new();
        let blocks = BTreeSet::from([5, 19]);
        p.ensure(&[&u], &blocks, 0, &mut maps);
        let look = p.look(&u, 0, 0).unwrap();
        assert_eq!(look.pal, CelPal::Shift(2));
        assert_eq!(maps.get(look.remap.unwrap()).unwrap()[0], 2);
        assert_eq!(p.look(&u, S7, 0), None);
        assert_eq!(p.look(&u, 0, 3), None);
        let other = monster(5, 0, 0, 0);
        p.ensure(&[&other], &blocks, 1, &mut maps);
        assert_eq!(p.look(&other, 0, 0), None, "no palshift.dat: no P");
    }

    #[test]
    fn client_only_class_has_no_block() {
        // class palshift data: the file exists, but no S→C 0xAC of
        // class 19 arrived (a client-only monster): no P.
        struct Files;
        impl FileSource for Files {
            fn read_file(&self, _: &str) -> Option<Result<Vec<u8>, String>> {
                Some(Ok(vec![7; 2048]))
            }
        }
        let mut p = PalShifts::new(Arc::new(rows()), Some(Arc::new(Files)));
        let u = monster(19, 0, 0, 0);
        let mut maps = MapTable::new();
        p.ensure(&[&u], &BTreeSet::new(), 0, &mut maps);
        assert_eq!(p.look(&u, 0, 0), None);
        // Once an 0xAC of the class arrives, its block (and so its map) is there.
        p.ensure(&[&u], &BTreeSet::from([19]), 0, &mut maps);
        assert_eq!(p.look(&u, 0, 0).unwrap().pal, CelPal::Shift(2));
    }
}
