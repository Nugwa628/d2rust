//! File-backed animation names for the headless game's mode starts.
//! Only table facts enter the simulation; no renderer or client model is needed.
use super::composite_names::{
    code, mode_overrides, mode_token, monster_weapon_class, Code, CofName, CompositeKind, HTH,
};
use crate::file_source::FileSource;
use crate::local_seams::UnitAppearance;
use d2_data::bin::{excel_path, BinTable};
use d2_data::tables::{
    decode_all, Monmode, Monstats, Monstats2, Objects, Objmode, Plrmode, Plrtype, Record,
};
use d2_sim::skills::sequences::CLASSES;
use d2_sim::units::UnitType;
use std::collections::BTreeMap;

/// The token and mode tables used to name a unit's animation.
#[derive(Debug, Clone, Default)]
pub struct AnimationNames {
    pub shields: BTreeMap<u32, u16>,
    pub players: Vec<Code>,
    pub player_modes: Vec<Code>,
    pub monsters: Vec<(Code, Option<Code>, bool)>,
    pub monster_modes: Vec<Code>,
    pub objects: Vec<Code>,
    pub object_modes: Vec<Code>,
}
fn read<T: Record>(source: &dyn FileSource) -> Result<Vec<T>, String> {
    let file = excel_path(&format!("{}.bin", T::TABLE));
    let bytes = source
        .read_file(&file)
        .ok_or_else(|| format!("{file}: in no archive"))??;
    let table =
        BinTable::parse(T::TABLE, "archive", &file, &bytes, T::SIZE).map_err(|e| e.to_string())?;
    decode_all(&table).map_err(|e| e.to_string())
}
fn padded(c: [u8; 4]) -> Code {
    let n = c.iter().position(|&b| b == 0).unwrap_or(4);
    code(&c[..n])
}
impl AnimationNames {
    /// Read the same token, mode and monster weapon columns as the client appearance table.
    pub fn load(source: &dyn FileSource) -> Result<Self, String> {
        let ex = read::<Monstats2>(source)?;
        let monsters = read::<Monstats>(source)?
            .iter()
            .map(|m| {
                let row = ex.get(usize::from(m.monstatsex));
                (
                    padded(m.code),
                    row.map(|r| padded(r.basew)).filter(|w| *w != code(b"")),
                    row.is_some_and(|r| r.compositedeath),
                )
            })
            .collect();
        Ok(Self {
            shields: shield_choices(source, &ex)?,
            players: read::<Plrtype>(source)?
                .iter()
                .map(|r| padded(r.token))
                .collect(),
            player_modes: read::<Plrmode>(source)?
                .iter()
                .map(|r| padded(r.token))
                .collect(),
            monster_modes: read::<Monmode>(source)?
                .iter()
                .map(|r| padded(r.token))
                .collect(),
            object_modes: read::<Objmode>(source)?
                .iter()
                .map(|r| padded(r.token))
                .collect(),
            objects: read::<Objects>(source)?
                .iter()
                .map(|o| padded([o.token[0], o.token[1], 0, 0]))
                .collect(),
            monsters,
        })
    }
    /// Name a unit without transient presentation state or equipped-item overrides.
    pub fn cof(&self, ty: UnitType, class: u32, mode: u32) -> Option<CofName> {
        let kind = CompositeKind::of_unit_type(ty as u32)?;
        let mode = u8::try_from(mode).ok()?;
        let class = class as usize;
        let (token, modes, weapon_class) = match kind {
            CompositeKind::Player => (*self.players.get(class)?, &self.player_modes, HTH),
            CompositeKind::Monster => {
                let &(token, base, death) = self.monsters.get(class)?;
                (
                    token,
                    &self.monster_modes,
                    monster_weapon_class(mode, death, base),
                )
            }
            CompositeKind::Object => (*self.objects.get(class)?, &self.object_modes, HTH),
        };
        if token == code(b"") || token == [0; 4] {
            return None;
        }
        let token_mode = mode_token(
            modes.get(usize::from(mode)).copied().unwrap_or([0; 4]),
            mode,
            mode_overrides(kind),
        );
        if token_mode == [0; 4] {
            return None;
        }
        Some(match kind {
            CompositeKind::Player => CofName::player(token, mode, token_mode, weapon_class),
            _ => CofName {
                kind,
                token,
                mode_token: token_mode,
                weapon_class,
            },
        })
    }
}
impl UnitAppearance for AnimationNames {
    fn shield_choices(&self, class: u32) -> Option<u16> {
        self.shields.get(&class).copied()
    }
    fn anim_key(&self, ty: UnitType, class: u32, mode: u32, weapon: i32) -> Option<[u8; 8]> {
        let mut cof = self.cof(ty, class, mode)?;
        if cof.kind == CompositeKind::Player && !cof.kind.is_death_mode(mode as u8) {
            let w = usize::try_from(weapon)
                .ok()
                .and_then(|i| CLASSES.get(i))
                .map_or(HTH, |c| code(c.as_bytes()));
            cof = CofName::player(cof.token, mode as u8, cof.mode_token, w);
        }
        let name = cof.short().to_ascii_uppercase();
        let bytes = name.as_bytes();
        (bytes.len() <= 8).then(|| {
            let mut key = [0; 8];
            key[..bytes.len()].copy_from_slice(bytes);
            key
        })
    }
}

/// Read each monster class's usable shield component choices.
pub fn shield_choices(
    source: &dyn FileSource,
    monstats2: &[Monstats2],
) -> Result<BTreeMap<u32, u16>, String> {
    let file = excel_path("monstats2.bin");
    let bytes = source
        .read_file(&file)
        .ok_or_else(|| format!("{file}: in no archive"))??;
    let raw = BinTable::parse("monstats2", "archive", &file, &bytes, Monstats2::SIZE)
        .map_err(|e| e.to_string())?;
    let comp: Vec<[u8; 4]> = read::<d2_data::tables::Compcode>(source)?
        .iter()
        .map(|r| r.code)
        .collect();
    let mut shield_codes: Vec<[u8; 4]> = Vec::new();
    for (code, ty) in read::<d2_data::tables::Armor>(source)?
        .iter()
        .map(|r| (r.code, r.type_))
        .chain(
            read::<d2_data::tables::Weapons>(source)?
                .iter()
                .map(|r| (r.code, r.type_)),
        )
        .chain(
            read::<d2_data::tables::Misc>(source)?
                .iter()
                .map(|r| (r.code, r.type_)),
        )
    {
        if ty == 2 {
            shield_codes.push(code);
        }
    }
    let mut out = BTreeMap::new();
    for (i, m) in read::<Monstats>(source)?.iter().enumerate() {
        let row = usize::from(m.monstatsex);
        if row >= monstats2.len() || row >= raw.count {
            continue;
        }
        let rec = raw.record(row);
        let mut mask = 0u16;
        for v in 0..12 {
            let c = usize::from(rec[38 + 12 * 7 + v]);
            let Some(code) = comp.get(c) else { continue };
            // Table holds one code, `tch `.
            if code == b"tch\0" || code == b"tch " {
                continue;
            }
            if shield_codes.contains(code) {
                mask |= 1 << v;
            }
        }
        if mask != 0 {
            out.insert(i as u32, mask);
        }
    }
    Ok(out)
}
