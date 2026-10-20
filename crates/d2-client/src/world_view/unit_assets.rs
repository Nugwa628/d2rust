//! Unit art for the play preview: the table facts a unit composite reads
//! ([`UnitLooks`]: unit tokens, mode tokens, component tokens, monster
//! base weapon class), the COF and component file names of a model unit
//! ([`unit_cof`], [`component_codes`]), and the loader that makes those
//! files resident from the user's archives ([`UnitArtLoader`]): the COF
//! into [`ViewAssets::cofs`], each direction of a component file into
//! [`ViewAssets::frames`], and the file facts the rules read
//! ([`UnitArt`], shared with [`super::unit_rules::UnitRules`]).
//!
//! Inputs the model lacks get the first playable preview's fills, each
//! marked
//! `D2RUST's own, unverified`: a player has no items (every armor class
//! `lit`, weapon class `hth`), a monster's
//! component choices are the spawn packet's (`MonsterData::components`).
//! A COF that fails to load is skipped with a log line, never retried; a
//! component file that is in no archive is the normal empty slot
//! (e.g. a player's SH `lit` with no shield):
//! remembered, no log line. A component file that is there but
//! does not parse is logged.
//!
//! Direction (settled; the turns go by steps): the facing
//! [`UnitArt::dir64`] reads is
//! the local player's predicted facing ([`UnitArt::pose_dir`], from
//! `bridge::predict`), else the unit's client path direction as the model
//! turns it ([`UnitArt::observe_facing`]: a monster's path copy;
//! a player's path turn). A unit the
//! model has turned by no path yet takes, D2RUST's own, the path
//! direction toward the target of a player's walk / run mode request
//! (codes 0x01 / 0x17 to a point, 0x00 / 0x18 to
//! a unit), else toward its last position change
//! other than a placement or a move of the local player; 0 when none is
//! known.

use d2_server::game::appearance::shield_choices;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, RwLock};

use d2_data::bin::{excel_path, BinTable};
use d2_data::tables::{
    decode_all, Armor, Composit, Itemtypes, Misc, Monmode, Monstats, Monstats2, Objects, Objmode,
    Plrmode, Plrtype, Record, States, Weapons,
};
use d2_formats::cof::{Cof, CofLayer};

use crate::assets::path::{CanonicalPath, FileSource};
use crate::bridge::predict::{cell_centre, facing};
use crate::bridge::world::{ClientWorld, KindData, UnitKey, PLAYER};
use crate::bridge::ClientUnit;
use crate::frames::{FramePart, FrameSet, FrameSetKey};
use crate::rules::unit_composite::{
    armor_class, code, expected_directions, file_direction, file_format, mode_overrides,
    mode_token, monster_weapon_class, player_weapon_class, ArmorSource, Code, CofName, CofPath,
    ComponentCodes, CompositeKind, DirectionSource, FileFormat, HandItem, HandLoc, MonsterLook,
    PlayerHands, PlayerLook, EMPTY, HTH,
};
use crate::rules::unit_visibility::CelBox;

use super::ViewAssets;

/// The `monstats` / `monstats2` facts of one monster class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterRow {
    /// `monstats` `Code` (unit token).
    pub token: Code,
    /// `monstats2` `BaseW`; `None`: no `monstats2` row.
    pub base_w: Option<Code>,
    /// `monstats2` `compositeDeath`.
    pub composite_death: bool,
    /// `monstats2` `d<mode>` (+0xF4 + mode) by mode, DT … RN; all
    /// 0 without a `monstats2` row.
    pub directions: [u8; 16],
}

/// The `monstats2` facts of the composite for one monster class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterComponents {
    /// Choice count per component (record +0x15 + c).
    pub counts: [u8; 16],
    /// The `compcode` code of choice v of component c (record
    /// +0x26 + 12c + v): `codes[c][v]`.
    pub codes: [[Code; 12]; 16],
}

/// The table facts of the composite for every unit kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitLooks {
    /// `plrtype` token by player class.
    pub player_tokens: Vec<Code>,
    /// `plrmode`, `monmode`, `objmode` token by mode.
    pub player_modes: Vec<Code>,
    pub monster_modes: Vec<Code>,
    pub object_modes: Vec<Code>,
    /// `composit` token by component id (rows 0–15).
    pub components: Vec<Code>,
    /// By `monstats` row.
    pub monsters: BTreeMap<u32, MonsterRow>,
    /// `objects` `Token` by row.
    pub objects: BTreeMap<u32, Code>,
    /// `objects` `BlocksLight0`–`7` by row: an object casts its composite
    /// shadow only in a mode whose value is ≠ 0. A row not here casts it.
    pub object_blocks_light: BTreeMap<u32, [u8; 8]>,
    /// The `objects` rows whose `Draw` (`+0x150`) is 0: the unit draw
    /// returns before any cel or shadow.
    pub object_no_draw: std::collections::BTreeSet<u32>,
    /// The shape states' draw identity.
    pub shapes: super::disguise::Disguise,
    /// By `monstats` row: bit v set when choice v of the `SH` component
    /// (slot 7) is a usable shield (not `tch `, and
    /// its items record has type 2).
    pub shield_choices: BTreeMap<u32, u16>,
    /// By `monstats` row: the `monstats2` choice counts and `compcode`
    /// codes the monster armor class reads.
    pub monster_components: BTreeMap<u32, MonsterComponents>,
    /// The items-table facts the player composite reads, by `code`:
    /// empty without the tables.
    pub items: BTreeMap<Code, ItemLook>,
}

/// The facts of one items record that the player composite reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemLook {
    /// `alternategfx` when set, else `code`.
    pub gfx: Code,
    /// `component`: 255 and the like match no layer.
    pub component: u8,
    /// The type class of the `wclass`: `bow` 1, `1hs` 2,
    /// `1ht` 3, `stf` 4, `2hs` 5, `2ht` 6, `xbw` 7, `ht1` 12, else 0.
    pub type_class: u8,
    /// Armor `torso`, `legs`, `rArm`, `lArm`, `rSPad`, `lSPad` (armor
    /// +0x116 … +0x11B); `None` for a weapon or misc record.
    pub armor: Option<[u8; 6]>,
    /// What reads of the item as a hand item.
    pub hand: HandItem,
    /// `transparent` (`+300`) and `transtbl` (`+301`): the draw mode of a
    /// component whose armor-class token is this item's code.
    pub trans: (u8, u8),
    /// The item is of item type 45 `weap` (its `type` or
    /// `type2`, `itemtypes` equivalence included): what the weapon in use
    /// takes.
    pub weapon: bool,
    /// The item is of item type 51 `ashd` (any shield, the same test): the
    /// shield hand item.
    pub shield: bool,
}

/// `itemtypes` rows of the item type tests: 45 `weap`, 51 `ashd`.
const ITYPE_WEAPON: usize = 45;
/// `states` row 101 `holyshield`.
const STATE_HOLYSHIELD: u8 = 101;
const ITYPE_ANY_SHIELD: usize = 51;

/// The type class of a `wclass` code (dual wield).
fn wclass_type(w: [u8; 4]) -> u8 {
    match &code4(w)[..3] {
        b"bow" => 1,
        b"1hs" => 2,
        b"1ht" => 3,
        b"stf" => 4,
        b"2hs" => 5,
        b"2ht" => 6,
        b"xbw" => 7,
        b"ht1" => 12,
        _ => 0,
    }
}

/// Weapons, armor and misc records by `code`: weapons first, as the base game's
/// record order, and a code seen first keeps its record.
fn item_looks(source: &dyn FileSource) -> Result<BTreeMap<Code, ItemLook>, String> {
    let mut out = BTreeMap::new();
    let file = excel_path("itemtypes.bin");
    let bytes = source
        .read_file(&file)
        .ok_or_else(|| format!("{file}: in no archive"))??;
    let types = BinTable::parse("itemtypes", "archive", &file, &bytes, Itemtypes::SIZE)
        .map_err(|e| e.to_string())?;
    let equiv =
        d2_data::fixup::maps::equiv_matrix(&types, d2_data::fixup::maps::EquivKind::ItemTypes)
            .map_err(|e| e.to_string())?;
    let is = |t: u16, t2: u16, j: usize| {
        equiv.get(usize::from(t), j) || (t2 != 0 && equiv.get(usize::from(t2), j))
    };
    let gfx = |alt: [u8; 4], c: [u8; 4]| if alt[0] != 0 { code4(alt) } else { code4(c) };
    for w in read_table::<Weapons>(source)? {
        out.entry(code4(w.code)).or_insert(ItemLook {
            gfx: gfx(w.alternategfx, w.code),
            component: w.component,
            type_class: wclass_type(w.wclass),
            hand: hand_item(
                w.component,
                w.type_,
                w.wclass,
                w.f_2handedwclass,
                w.f_2handed,
            ),
            trans: (w.transparent, w.transtbl),
            weapon: is(w.type_, w.type2, ITYPE_WEAPON),
            shield: is(w.type_, w.type2, ITYPE_ANY_SHIELD),
            armor: None,
        });
    }
    for a in read_table::<Armor>(source)? {
        out.entry(code4(a.code)).or_insert(ItemLook {
            gfx: gfx(a.alternategfx, a.code),
            component: a.component,
            type_class: wclass_type(a.wclass),
            hand: hand_item(
                a.component,
                a.type_,
                a.wclass,
                a.f_2handedwclass,
                a.f_2handed,
            ),
            trans: (a.transparent, a.transtbl),
            weapon: is(a.type_, a.type2, ITYPE_WEAPON),
            shield: is(a.type_, a.type2, ITYPE_ANY_SHIELD),
            armor: Some([a.torso, a.legs, a.rarm, a.larm, a.rspad, a.lspad]),
        });
    }
    for m in read_table::<Misc>(source)? {
        out.entry(code4(m.code)).or_insert(ItemLook {
            gfx: gfx(m.alternategfx, m.code),
            component: m.component,
            type_class: wclass_type(m.wclass),
            hand: hand_item(
                m.component,
                m.type_,
                m.wclass,
                m.f_2handedwclass,
                m.f_2handed,
            ),
            trans: (m.transparent, m.transtbl),
            weapon: is(m.type_, m.type2, ITYPE_WEAPON),
            shield: is(m.type_, m.type2, ITYPE_ANY_SHIELD),
            armor: None,
        });
    }
    Ok(out)
}

/// The grip test reads the `2handed` column (record
/// +0x11C) and, for a Barbarian,
/// `1or2handed` (+0x13D), as `app::weapons::hands_grip`.
fn hand_item(
    component: u8,
    item_type: u16,
    wclass: [u8; 4],
    wclass2: [u8; 4],
    two: u8,
) -> HandItem {
    HandItem {
        component,
        item_type,
        wclass: code4(wclass),
        two_handed_wclass: code4(wclass2),
        two_handed_grip: two != 0,
    }
}

/// `objects` Xoffset / Yoffset (`+0x148` / `+0x14C`) by row, rows with
/// both 0 left out: the object draw adds them to the screen position
/// before the COF box pre-test.
pub fn object_offsets(source: &dyn FileSource) -> Result<BTreeMap<u32, (i32, i32)>, String> {
    Ok(read_table::<Objects>(source)?
        .iter()
        .enumerate()
        .filter(|(_, o)| (o.xoffset, o.yoffset) != (0, 0))
        .map(|(i, o)| (i as u32, (o.xoffset as i32, o.yoffset as i32)))
        .collect())
}

pub(crate) fn read_table<T: Record>(source: &dyn FileSource) -> Result<Vec<T>, String> {
    let file = excel_path(&format!("{}.bin", T::TABLE));
    let bytes = source
        .read_file(&file)
        .ok_or_else(|| format!("{file}: in no archive"))??;
    let table =
        BinTable::parse(T::TABLE, "archive", &file, &bytes, T::SIZE).map_err(|e| e.to_string())?;
    decode_all(&table).map_err(|e| e.to_string())
}

/// The `monstats2` mode bits (mDT = bit 0 … mRN = bit 15) by monster row.
fn monstats2_modes(monstats: &[Monstats], monstats2: &[Monstats2]) -> BTreeMap<u32, u16> {
    monstats
        .iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let r = monstats2.get(usize::from(m.monstatsex))?;
            let bits = [
                r.mdt, r.mnu, r.mwl, r.mgh, r.ma1, r.ma2, r.mbl, r.msc, r.ms1, r.ms2, r.ms3, r.ms4,
                r.mdd, r.mkb, r.msq, r.mrn,
            ]
            .iter()
            .enumerate()
            .fold(0u16, |a, (b, &on)| a | (u16::from(on) << b));
            Some((i as u32, bits))
        })
        .collect()
}

fn code4(c: [u8; 4]) -> Code {
    // `.bin` codes are zero padded; the composite codes are space padded.
    let n = c.iter().position(|&b| b == 0).unwrap_or(4);
    code(&c[..n])
}

/// The monster composite facts by `monstats` row (see
/// [`UnitLooks::monster_components`]).
fn monster_components(
    source: &dyn FileSource,
    monstats2: &[Monstats2],
) -> Result<BTreeMap<u32, MonsterComponents>, String> {
    let file = excel_path("monstats2.bin");
    let bytes = source
        .read_file(&file)
        .ok_or_else(|| format!("{file}: in no archive"))??;
    let raw = BinTable::parse("monstats2", "archive", &file, &bytes, Monstats2::SIZE)
        .map_err(|e| e.to_string())?;
    let comp: Vec<Code> = read_table::<d2_data::tables::Compcode>(source)?
        .iter()
        .map(|r| code4(r.code))
        .collect();
    let mut out = BTreeMap::new();
    for (i, m) in read_table::<Monstats>(source)?.iter().enumerate() {
        let row = usize::from(m.monstatsex);
        if row >= monstats2.len() || row >= raw.count {
            continue;
        }
        let rec = raw.record(row);
        let mut counts = [0u8; 16];
        counts.copy_from_slice(&rec[0x15..0x25]);
        let mut codes = [[EMPTY; 12]; 16];
        for (c, per) in codes.iter_mut().enumerate() {
            for (v, slot) in per.iter_mut().enumerate() {
                if let Some(&k) = comp.get(usize::from(rec[38 + 12 * c + v])) {
                    *slot = k;
                }
            }
        }
        out.insert(i as u32, MonsterComponents { counts, codes });
    }
    Ok(out)
}

impl UnitLooks {
    /// The facts from the user's `.bin` tables (`data\global\excel`).
    pub fn live(source: &dyn FileSource) -> Result<Self, String> {
        let monstats2: Vec<Monstats2> = read_table(source)?;
        let monsters = read_table::<Monstats>(source)?
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let row = monstats2.get(usize::from(m.monstatsex));
                let base_w = row.map(|r| code4(r.basew)).filter(|w| *w != code(b""));
                (
                    i as u32,
                    MonsterRow {
                        token: code4(m.code),
                        base_w,
                        composite_death: row.is_some_and(|r| r.compositedeath),
                        directions: row.map_or([0; 16], |r| {
                            [
                                r.ddt, r.dnu, r.dwl, r.dgh, r.da1, r.da2, r.dbl, r.dsc, r.ds1,
                                r.ds2, r.ds3, r.ds4, r.ddd, r.dkb, r.dsq, r.drn,
                            ]
                        }),
                    },
                )
            })
            .collect();
        let shapes = super::disguise::Disguise {
            states: read_table::<States>(source)?
                .iter()
                .enumerate()
                .filter(|(_, r)| matches!(r.gfxtype, 1 | 2))
                .map(|(i, r)| (i as u8, r.gfxtype, r.gfxclass))
                .collect(),
            mode_bits: monstats2_modes(&read_table::<Monstats>(source)?, &monstats2),
        };
        let object_rows = read_table::<Objects>(source)?;
        let objects = object_rows
            .iter()
            .enumerate()
            .map(|(i, o)| (i as u32, code4([o.token[0], o.token[1], 0, 0])))
            .collect();
        let object_blocks_light = object_rows
            .iter()
            .enumerate()
            .map(|(i, o)| {
                (
                    i as u32,
                    [
                        o.blockslight0,
                        o.blockslight1,
                        o.blockslight2,
                        o.blockslight3,
                        o.blockslight4,
                        o.blockslight5,
                        o.blockslight6,
                        o.blockslight7,
                    ],
                )
            })
            .collect();
        let object_no_draw = object_rows
            .iter()
            .enumerate()
            .filter(|(_, o)| o.draw == 0)
            .map(|(i, _)| i as u32)
            .collect();
        let shield_choices = shield_choices(source, &monstats2)?;
        let monster_components = monster_components(source, &monstats2)?;
        Ok(UnitLooks {
            player_tokens: read_table::<Plrtype>(source)?
                .iter()
                .map(|r| code4(r.token))
                .collect(),
            player_modes: read_table::<Plrmode>(source)?
                .iter()
                .map(|r| code4(r.token))
                .collect(),
            monster_modes: read_table::<Monmode>(source)?
                .iter()
                .map(|r| code4(r.token))
                .collect(),
            object_modes: read_table::<Objmode>(source)?
                .iter()
                .map(|r| code4(r.token))
                .collect(),
            components: read_table::<Composit>(source)?
                .iter()
                .map(|r| code4(r.token))
                .collect(),
            monsters,
            objects,
            object_blocks_light,
            object_no_draw,
            shapes,
            shield_choices,
            monster_components,
            items: item_looks(source)?,
        })
    }

    fn modes(&self, kind: CompositeKind) -> &[Code] {
        match kind {
            CompositeKind::Player => &self.player_modes,
            CompositeKind::Monster => &self.monster_modes,
            CompositeKind::Object => &self.object_modes,
        }
    }
}

/// The unit's composite kind and the `u8` mode the rules read.
fn kind_mode(unit: &ClientUnit) -> Option<(CompositeKind, u8)> {
    let kind = CompositeKind::of_unit_type(u32::from(unit.key.unit_type))?;
    Some((kind, u8::try_from(unit.mode).ok()?))
}

/// weapon class. Players: `hth` (preview fill: the model holds no
/// equipped items; `// D2RUST's own, unverified`). Objects: `hth`.
fn weapon_class(looks: &UnitLooks, unit: &ClientUnit, kind: CompositeKind, mode: u8) -> Code {
    match kind {
        CompositeKind::Monster => {
            let row = looks.monsters.get(&unit.class);
            monster_weapon_class(
                mode,
                row.is_some_and(|r| r.composite_death),
                row.and_then(|r| r.base_w),
            )
        }
        // D2RUST's own, unverified (the preview): no items, bare hands.
        CompositeKind::Player | CompositeKind::Object => HTH,
    }
}

/// unit token.
fn unit_token(looks: &UnitLooks, unit: &ClientUnit, kind: CompositeKind) -> Option<Code> {
    let t = match kind {
        CompositeKind::Player => looks.player_tokens.get(unit.class as usize).copied(),
        CompositeKind::Monster => looks.monsters.get(&unit.class).map(|r| r.token),
        CompositeKind::Object => looks.objects.get(&unit.class).copied(),
    }?;
    (t != code(b"") && t != EMPTY).then_some(t)
}

/// The mode token of the composite (empty: none), before the mode override.
fn base_mode_token(looks: &UnitLooks, kind: CompositeKind, mode: u8) -> Code {
    looks
        .modes(kind)
        .get(usize::from(mode))
        .copied()
        .unwrap_or(EMPTY)
}

/// The COF name of a model unit, `None` for a unit without a
/// composite (types 3–5, unknown class or mode).
pub fn unit_cof(looks: &UnitLooks, unit: &ClientUnit) -> Option<CofName> {
    unit_cof_with(looks, unit, None)
}

/// [`unit_cof`] with a player's weapon class `hands` (none: `hth`).
pub fn unit_cof_with(looks: &UnitLooks, unit: &ClientUnit, hands: Option<Code>) -> Option<CofName> {
    let unit = &*looks.shapes.identity(unit);
    let (kind, mode) = kind_mode(unit)?;
    let token = unit_token(looks, unit, kind)?;
    let m = mode_token(
        base_mode_token(looks, kind, mode),
        mode,
        mode_overrides(kind),
    );
    if m == EMPTY {
        return None;
    }
    let mut w = weapon_class(looks, unit, kind, mode);
    if kind == CompositeKind::Player && !kind.is_death_mode(mode) {
        w = hands.unwrap_or(w);
    }
    Some(match kind {
        CompositeKind::Player => CofName::player(token, mode, m, w),
        _ => CofName {
            kind,
            token,
            mode_token: m,
            weapon_class: w,
        },
    })
}

/// What reads of a player's equipped items: the body armor bytes
/// (location 3) and, per component: walking the body slots
/// 1..=11 in order, an item whose `wclass` type class is 2, 3 or 12 is the
/// component 5 item when it is the weapon in use (the right hand item,
/// location 4) and the component 6 item when it is not; any other item is
/// the item of its own `component`; `shield_hand_item` is a shield in
/// location 4 or 5 (`holy_shield`, state 101, is the
/// unit's and is set by [`component_codes`]).
/// for a player's equipped items: the hand items of body locations
/// 4 and 5. `in_use` is the body location of the weapon in use
/// ([`UnitArt::observe_equipment`]).
fn player_hands_class(
    looks: &UnitLooks,
    class: u8,
    equipped: &[(u8, Code)],
    in_use: Option<u8>,
) -> Code {
    let at = |loc: u8| {
        equipped
            .iter()
            .find(|(b, _)| *b == loc)
            .and_then(|(_, c)| looks.items.get(c))
            .map(|l| l.hand)
    };
    let (loc4, loc5) = (at(4), at(5));
    player_weapon_class(&PlayerHands {
        class,
        base_wclass: HTH,
        loc4,
        loc5,
        in_use: match in_use {
            Some(4) => Some(HandLoc::Loc4),
            Some(5) => Some(HandLoc::Loc5),
            _ => None,
        },
    })
    .unwrap_or(HTH)
}

pub fn player_look(looks: &UnitLooks, equipped: &[(u8, Code)], in_use: Option<u8>) -> PlayerLook {
    let mut slots: Vec<(u8, ItemLook)> = equipped
        .iter()
        .filter(|(b, _)| (1..=11).contains(b))
        .filter_map(|(b, c)| looks.items.get(c).map(|l| (*b, *l)))
        .collect();
    slots.sort_by_key(|(b, _)| *b);
    let in_use = slots.iter().position(|(b, _)| Some(*b) == in_use);
    let mut item_gfx = [None; 16];
    for (c, out) in item_gfx.iter_mut().enumerate() {
        let c = c as u8;
        for (i, (_, l)) in slots.iter().enumerate() {
            if matches!(l.type_class, 2 | 3 | 12) {
                let hit = match c {
                    6 => in_use != Some(i),
                    5 => in_use == Some(i),
                    _ => false,
                };
                if hit {
                    *out = Some(l.gfx);
                    break;
                }
            } else if l.component == c {
                *out = Some(l.gfx);
                break;
            }
        }
    }
    PlayerLook {
        body_armor: slots
            .iter()
            .find(|(b, _)| *b == 3)
            .and_then(|(_, l)| l.armor),
        item_gfx,
        holy_shield: false,
        shield_hand_item: slots.iter().any(|(b, l)| matches!(b, 4 | 5) && l.shield),
    }
}

/// The component request of the COF layer `layer`, `None` when it fails (the
/// slot draws nothing). `equip` is the player's equipped items
/// ([`player_look`]); without it (preview fill, `// D2RUST's own,
/// unverified`) a player has no items and no body armor; a monster's choices
/// are the spawn packet's.
pub fn component_codes(
    looks: &UnitLooks,
    unit: &ClientUnit,
    name: &CofName,
    layer: &CofLayer,
    equip: Option<&PlayerLook>,
) -> Option<ComponentCodes> {
    let unit = &*looks.shapes.identity(unit);
    let (kind, mode) = kind_mode(unit)?;
    let c = layer.component;
    let component_token = *looks.components.get(usize::from(c))?;
    let mut player = equip.copied().unwrap_or(PlayerLook {
        body_armor: None,
        item_gfx: [None; 16],
        holy_shield: false,
        shield_hand_item: false,
    });
    // State 101 `holyshield` of the drawn unit.
    player.holy_shield = unit.states.contains(&STATE_HOLYSHIELD);
    // The choices are the spawn packet's; a unit with
    // no monster data, or a class with no `monstats2` row, keeps every
    // component `lit`.
    let choices = match &unit.kind {
        KindData::Monster(m) => m.components,
        _ => [0; 16],
    };
    let facts = looks.monster_components.get(&unit.class);
    let monster = MonsterLook {
        choices,
        counts: facts.map_or([0; 16], |f| f.counts),
        codes: facts.map_or([[EMPTY; 12]; 16], |f| f.codes),
        composite_death: looks
            .monsters
            .get(&unit.class)
            .is_some_and(|r| r.composite_death),
        act_two: false,
        base_class: unit.class,
    };
    let source = match kind {
        CompositeKind::Player => ArmorSource::Player(Some(&player)),
        CompositeKind::Monster => ArmorSource::Monster(&monster),
        CompositeKind::Object => ArmorSource::Object,
    };
    let armor = armor_class(source, c, mode, name.weapon_class).ok()?;
    ComponentCodes::new(
        kind,
        name.token,
        component_token,
        armor,
        base_mode_token(looks, kind, mode),
        layer_weapon_class(layer),
    )
}

/// The weapon class of a component file name:
/// the COF layer's own (`weaponClass`,
/// nul-padded in the file), not the unit's class: the bytes up to
/// the first 0, the rest spaces, so an empty class is four spaces and
/// the name has no weapon-class part.
fn layer_weapon_class(layer: &CofLayer) -> Code {
    let mut c = *b"    ";
    for (d, s) in c.iter_mut().zip(layer.weapon_class.iter().take(3)) {
        if *s == 0 {
            break;
        }
        *d = *s;
    }
    c
}

/// One loaded component file: its format, directions `Df` and frames per
/// direction `Ff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFacts {
    pub format: FileFormat,
    pub directions: u8,
    pub frames: usize,
}

/// What the loader made resident, read by the rules (they get no
/// [`ViewAssets`] in `unit_pose` / `component_slot_frame`).
#[derive(Debug, Clone, Default)]
pub struct UnitArt {
    /// Loaded COFs (also in [`ViewAssets::cofs`]).
    pub cofs: BTreeMap<CanonicalPath, Cof>,
    /// Component files by request name (no extension): `None` =
    /// tried and missing or refused (the slot draws nothing).
    pub files: BTreeMap<String, Option<(CanonicalPath, FileFacts)>>,
    /// COF paths tried and missing or refused.
    pub missing_cofs: BTreeSet<CanonicalPath>,
    /// The mode the local player is drawn in while the play preview's
    /// walk prediction moves it (`bridge::predict`: 2 walk,
    /// 3 run). D2RUST's own, unverified.
    pub pose_mode: Option<(UnitKey, u32)>,
    /// The local player's predicted facing `dir64` (`Predict::facing`),
    /// set with [`Self::pose_mode`]. D2RUST's own, unverified.
    pub pose_dir: Option<(UnitKey, u8)>,
    /// The unit in mode 18 and the sequence frame it draws
    /// (`world_view::skill_motion`: the drawn
    /// mode goes in [`Self::pose_mode`]).
    pub sequence: Option<(UnitKey, usize)>,
    /// The server tick the local player's predicted walk started on
    /// (`Predict::walk_since`): its walk frames count from there.
    /// With the animation speed (8.8 per tick) of the predicted mode.
    pub pose_since: Option<(UnitKey, u64, u64)>,
    /// The model facing of every unit (module doc), by
    /// [`Self::observe_facing`].
    pub facing: BTreeMap<UnitKey, Facing>,
    /// The model's act loads (`ClientWorld::act_loads`) at the last
    /// [`Self::observe_facing`].
    pub facing_act_loads: u64,
    /// The model's local player at the last [`Self::observe_facing`].
    pub local: Option<UnitKey>,
    /// The drawn frame's monster graphics-ready flags
    /// ([`ClientWorld::gfx_ready_now`] at [`UnitArtLoader::ensure`]);
    /// `None`: not recorded, every class draws as ready.
    pub gfx_ready: Option<crate::bridge::world::GfxReady>,
    /// Each player's equipped items as the composite reads them, by
    /// [`Self::observe_equipment`].
    pub equip: BTreeMap<UnitKey, PlayerLook>,
    /// Each player's COF weapon class, by [`Self::observe_equipment`].
    pub hands: BTreeMap<UnitKey, Code>,
    /// Each player's weapon in use (inventory +0x1C): the
    /// item, by [`Self::observe_equipment`].
    pub in_use: BTreeMap<UnitKey, UnitKey>,
    /// The cel fields of every loaded component file, by file direction
    /// and frame: what the visibility
    /// predicate's cel box test reads (`super::visibility`).
    pub cels: BTreeMap<CanonicalPath, Vec<Vec<CelBox>>>,
    /// The direction blocks of each component DCC the resolver has asked
    /// for ([`Self::block_dir64`]); shared so a draw (read lock)
    /// can record its request.
    pub blocks: Arc<Mutex<DccBlocks>>,
}

/// The direction-block load state of the component DCCs:
/// a block the resolver asks for in
/// one client update's frame is loaded from the next update's frame on
/// (the block cache's asynchronous read; block 0 never
/// falls back). Measured: the Fallen corpses' first
/// DD frame draws `FAS8litDDhth` dir 47 with the dir-40 cel,
/// the dir-47 one. Bit `b` of a mask is block `b`.
#[derive(Debug, Clone, Default)]
pub struct DccBlocks {
    loaded: BTreeMap<CanonicalPath, u32>,
    asked: BTreeMap<CanonicalPath, u32>,
    /// The update the asks were made in.
    tick: u64,
}

impl DccBlocks {
    /// The frame of update `tick`: on a new update the blocks asked for
    /// before are loaded (more frames of one update ask in the same one).
    pub fn settle(&mut self, tick: u64) {
        if tick == self.tick {
            return;
        }
        self.tick = tick;
        for (path, bits) in std::mem::take(&mut self.asked) {
            *self.loaded.entry(path).or_default() |= bits;
        }
    }
}

/// The fallback table for (`Df` 8, `n` 4), flag clear.
const BLOCK_STEP_8_4: [u8; 8] = [1, 1, 3, 3, 5, 5, 7, 7];

/// A unit's facing as the view last saw it (module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facing {
    /// The model position at the last observation.
    pub at: (u16, u16),
    /// `dir64` (0–63).
    pub dir64: u8,
    /// [`ClientUnit::placements`] at that observation.
    pub placements: u32,
}

/// The cell a player's walk / run mode request goes to (:
/// codes 0x01 / 0x17 to the point, 0x00 / 0x18 to the
/// unit (type, GUID)). A 0x13 soft hit
/// resets no path and sets no mode in the base game: the
/// request before it still sets the target ([`ClientUnit::last_path_request`]).
fn request_target(world: &ClientWorld, unit: &ClientUnit) -> Option<(u16, u16)> {
    if unit.key.unit_type != PLAYER {
        return None;
    }
    let (r, _) = unit.last_path_request.as_ref()?;
    match r.code {
        0x01 | 0x17 => Some((
            u16::try_from(r.record[0]).ok()?,
            u16::try_from(r.record[1]).ok()?,
        )),
        0x00 | 0x18 => {
            let key = UnitKey::new(u8::try_from(r.record[0]).ok()?, r.record[1] as u32);
            world.units.get(&key)?.position
        }
        // The client skill start with (unit, x, y) faces the cast point;
        // a unit target is faced the same way. The face turns by steps:
        // the cast sets the player's
        // pending turn (`critter_path::face_point`) and
        // [`UnitArt::observe_facing`] draws its stepped path direction;
        // this whole-turn target is only the fallback for a player with no
        // pending turn (no path tables loaded).
        0x15 => Some((
            u16::try_from(r.record[2]).ok()?,
            u16::try_from(r.record[3]).ok()?,
        )),
        0x16 => {
            let key = UnitKey::new(u8::try_from(r.record[2]).ok()?, r.record[3] as u32);
            world.units.get(&key)?.position
        }
        _ => None,
    }
}

impl UnitArt {
    /// `unit` with its drawn mode ([`Self::pose_mode`] for its key; a
    /// monster with a sequence: the sequence's drawn mode +0x40;
    /// else the model's).
    pub fn posed<'a>(&self, unit: &'a ClientUnit) -> std::borrow::Cow<'a, ClientUnit> {
        if let Some(seq) = unit.sequence.as_ref().filter(|s| s.drawn_mode != unit.mode) {
            let mut u = unit.clone();
            u.mode = seq.drawn_mode;
            return std::borrow::Cow::Owned(u);
        }
        match self.pose_mode {
            Some((key, mode)) if key == unit.key && unit.mode != mode => {
                let mut u = unit.clone();
                u.mode = mode;
                std::borrow::Cow::Owned(u)
            }
            _ => std::borrow::Cow::Borrowed(unit),
        }
    }
}

impl UnitArt {
    /// The equipped items of player `key` (none known: no items).
    pub fn hands_of(&self, key: UnitKey) -> Option<Code> {
        self.hands.get(&key).copied()
    }

    pub fn equip_of(&self, key: UnitKey) -> Option<&PlayerLook> {
        self.equip.get(&key)
    }

    /// Updates [`Self::equip`] from the model's items: every item in a
    /// body slot (mode 1) of a player, from its last record
    /// (`bridge::items`; the owner of a 0x9C is the local player). A hand
    /// item (location 4 or 5) that fails (item flag 0x100 or
    /// 0x4000: broken, or requirements not met, the flag word of its
    /// record) is not taken: the weapon
    /// class reads only hand items that pass it, and the
    /// hand's art is then `lit` (measured: a level-67 claw on a level-30
    /// assassin draws `rh` `lit` with class `hth`).
    pub fn observe_equipment(&mut self, world: &ClientWorld, looks: &UnitLooks) {
        let mut by_owner: BTreeMap<UnitKey, Vec<(u8, Code, UnitKey)>> = BTreeMap::new();
        for i in crate::bridge::items::items(world) {
            let (Some(owner), Some(c)) = (i.owner, i.code) else {
                continue;
            };
            let model = match world.units.get(&i.key).map(|u| &u.kind) {
                Some(KindData::Item(d)) => d.flags,
                _ => 0,
            };
            if matches!(i.body, 4 | 5) && (i.flags | model) & (0x100 | 0x4000) != 0 {
                continue;
            }
            if i.mode == crate::bridge::items::mode::BODY && !i.store {
                by_owner
                    .entry(owner)
                    .or_default()
                    .push((i.body, code4(c), i.key));
            }
        }
        // The weapon in use (from the client's item action
        // handlers, r-equip): a valid weapon put in
        // location 4 or 5 becomes it when there is none or the current one
        // is not a weapon; it ends with its item leaving the hands. Items
        // the model shows in one update are taken in key (GUID) order, the
        // order a save's items arrive in.
        let weapon = |c: &Code| looks.items.get(c).is_some_and(|l| l.weapon);
        let mut in_use = BTreeMap::new();
        for (owner, v) in &by_owner {
            let held = |k: &UnitKey| {
                v.iter()
                    .find(|(b, c, i)| i == k && matches!(b, 4 | 5) && weapon(c))
                    .map(|_| *k)
            };
            let kept = self.in_use.get(owner).and_then(held);
            let first = || {
                v.iter()
                    .find(|(b, c, _)| matches!(b, 4 | 5) && weapon(c))
                    .map(|(_, _, i)| *i)
            };
            if let Some(k) = kept.or_else(first) {
                in_use.insert(*owner, k);
            }
        }
        self.in_use = in_use;
        let loc = |owner: &UnitKey, v: &[(u8, Code, UnitKey)]| {
            let k = self.in_use.get(owner)?;
            v.iter().find(|(_, _, i)| i == k).map(|(b, _, _)| *b)
        };
        self.hands = by_owner
            .iter()
            .map(|(k, v)| {
                let class = world
                    .view_unit(k)
                    .map_or(0, |u| u.class.try_into().unwrap_or(0));
                let items: Vec<(u8, Code)> = v.iter().map(|(b, c, _)| (*b, *c)).collect();
                (*k, player_hands_class(looks, class, &items, loc(k, v)))
            })
            .collect();
        self.equip = by_owner
            .iter()
            .map(|(k, v)| {
                let items: Vec<(u8, Code)> = v.iter().map(|(b, c, _)| (*b, *c)).collect();
                (*k, player_look(looks, &items, loc(k, v)))
            })
            .collect();
    }

    /// Updates [`Self::facing`] from the model (module doc; D2RUST's own,
    /// unverified): a C monster's path direction (`ClientWorld::view_direction`,
    /// the base-game unit's own); else toward a player's walk target when it has one and is
    /// not on it, else toward the last position change; otherwise the
    /// facing stays. Units no longer in the model are dropped.
    pub fn observe_facing(&mut self, world: &ClientWorld) {
        self.local = world.local_player;
        // 0x03 frees the client act with its units and 0x59 builds a new
        // local player record (path direction 0): no facing carries over
        // an act load, so the new record is not turned toward its new
        // position (measured: the player keeps
        // direction 0 after the act 2 → 3 warp).
        if self.facing_act_loads != world.act_loads {
            self.facing_act_loads = world.act_loads;
            self.facing.clear();
        }
        self.facing.retain(|k, _| world.view_unit(k).is_some());
        for unit in world.view_units() {
            let unit = &*unit;
            let Some(pos) = unit.position else { continue };
            let old = self.facing.get(&unit.key).copied();
            let toward = |to: (u16, u16)| facing(cell_centre(pos), cell_centre(to));
            let dir = world
                .view_direction(&unit.key)
                // a server monster draws with the direction its client
                // path copy turns
                .or_else(|| {
                    (unit.key.unit_type == crate::bridge::world::MONSTER)
                        .then(|| {
                            crate::bridge::critter_path::direction(
                                world,
                                crate::bridge::critter_path::Who::server(unit.key),
                            )
                        })
                        .flatten()
                })
                // a player draws with its client path facing once the
                // client turns it (+0x64: the walk's aim and the cast's
                // face, both turned by the player update)
                .or_else(|| {
                    (unit.key.unit_type == crate::bridge::world::PLAYER && unit.path_turn.is_some())
                        .then_some(unit.path_dir)
                        .flatten()
                })
                .or_else(|| request_target(world, unit).and_then(toward))
                // a 0x15 placement is a teleport: the base game turns no unit by it.
                // Nor the local player at all: its drawn direction is its
                // path's +0x64, which only its walk and the player update
                // turn (above); the single-player copy that moves it to a
                // placed point runs before the
                // 0x15 reaches the client and leaves +0x64 alone.
                .or_else(|| {
                    old.filter(|f| f.placements == unit.placements)
                        .filter(|_| world.local_player != Some(unit.key))
                        .and_then(|f| facing(cell_centre(f.at), cell_centre(pos)))
                })
                .or(old.map(|f| f.dir64))
                .or(unit.path_dir)
                .unwrap_or(0);
            self.facing.insert(
                unit.key,
                Facing {
                    at: pos,
                    dir64: dir,
                    placements: unit.placements,
                },
            );
        }
    }

    /// The `dir64` the cel resolver draws a component file
    /// with: for a monster DCC of `Df` 8 (blocks of `n` = 4 file
    /// directions) whose block of `dir64` is not 0 and not loaded yet, the
    /// loaded direction `BLOCK_STEP_8_4[(dir64 × 8) >> 6] × 8`, and the
    /// block is asked for (loaded from the next frame, [`DccBlocks`]);
    /// else `dir64`. Players and the other `n` are not measured yet: they
    /// keep `dir64` (the file header's bit-0 exemption is not read either).
    pub fn block_dir64(
        &self,
        path: &CanonicalPath,
        kind: CompositeKind,
        facts: FileFacts,
        dir64: u8,
    ) -> u8 {
        const N: u8 = 4;
        if kind != CompositeKind::Monster
            || facts.format != FileFormat::Dcc
            || facts.directions != 8
        {
            return dir64;
        }
        let Ok(block) = file_direction(8, dir64).map(|d| d / N) else {
            return dir64;
        };
        if block == 0 {
            return dir64;
        }
        let bit = 1u32 << block;
        let mut b = self.blocks.lock().unwrap_or_else(|e| e.into_inner());
        if b.loaded.get(path).is_some_and(|m| m & bit != 0) {
            return dir64;
        }
        *b.asked.entry(path.clone()).or_default() |= bit;
        let step = (usize::from(dir64) * 8) >> 6;
        // dir64 = step' × 64 / Df (< 64).
        BLOCK_STEP_8_4[step & 7] * (64 / 8)
    }

    /// The `dir64` the view draws `unit` with (module doc): the predicted
    /// facing of the local player, else the observed facing, else 0.
    pub fn dir64(&self, unit: &ClientUnit) -> u8 {
        match self.pose_dir {
            Some((key, d)) if key == unit.key => d,
            _ => self.facing.get(&unit.key).map_or(0, |f| f.dir64),
        }
    }

    /// expected direction count `n` of `unit` drawn with a COF of
    /// `d` directions: players 8, the local player 16 (= 0
    /// on a full install, open question 8); objects 1. Monsters: 4 when
    /// `monstats2` `d<mode>` is 8 and the frame's recorded graphics-ready
    /// flag of the class and mode is 0 (case 1, the war-cry
    /// measurement), else `d` (D2RUST's own, unverified: a ready
    /// class takes `d<mode>`, equal to `d` for the classes measured).
    pub fn expected_directions(
        &self,
        looks: &UnitLooks,
        unit: &ClientUnit,
        kind: CompositeKind,
        d: u8,
    ) -> u8 {
        let mode = i32::try_from(unit.mode).unwrap_or(-1);
        match kind {
            CompositeKind::Player => expected_directions(
                DirectionSource::Player {
                    sixteen: self.local == Some(unit.key),
                },
                mode,
            ),
            CompositeKind::Monster => {
                let d_mode = looks
                    .monsters
                    .get(&unit.class)
                    .and_then(|r| r.directions.get(usize::try_from(unit.mode).ok()?).copied());
                match &self.gfx_ready {
                    Some(ready) if d_mode == Some(8) && !ready.ready(unit.class, unit.mode) => {
                        expected_directions(
                            DirectionSource::Monster {
                                d_mode: 8,
                                mode_table_zero: true,
                            },
                            mode,
                        )
                    }
                    _ => d,
                }
            }
            CompositeKind::Object => expected_directions(DirectionSource::Other, mode),
        }
    }
}

pub type SharedUnitArt = Arc<RwLock<UnitArt>>;

/// Reads the unit files the model's units name, once each.
pub struct UnitArtLoader {
    pub source: Arc<dyn FileSource>,
    pub looks: Arc<UnitLooks>,
    pub art: SharedUnitArt,
}

impl UnitArt {
    /// The cel of `unit`'s COF layer of component `component` at its
    /// direction and frame `+0x44 >> 8` (the component request and its
    /// cel fields); `None` when the unit has no COF loaded for its draw
    /// mode, the COF has no such layer, the component request fails, or
    /// the file or cel did not load. Reads only what [`UnitArtLoader::ensure`]
    /// made resident.
    pub fn layer_cel(&self, looks: &UnitLooks, unit: &ClientUnit, component: u8) -> Option<CelBox> {
        use crate::rules::unit_composite::{component_cel, frame_index, unit_direction};
        let posed = self.posed(unit);
        let unit = &*looks.shapes.identity(&posed);
        let name = unit_cof_with(looks, unit, self.hands_of(unit.key))?;
        let cof = self.cofs.get(&name.path().ok()?)?;
        let layer = cof.layers.iter().find(|l| l.component == component)?;
        let codes = component_codes(looks, unit, &name, layer, self.equip_of(unit.key))?;
        let (path, facts) = self.files.get(&codes.name())?.as_ref()?;
        let n = self.expected_directions(looks, unit, name.kind, cof.directions);
        let dir = unit_direction(cof.directions, n, self.dir64(unit), false).ok()?;
        let frame = frame_index(unit.frame as u32);
        let cel = component_cel(path, facts.directions, facts.frames, dir.dir64, frame).ok()?;
        let crate::frames::FramePart::Dir(d) = cel.set.part() else {
            return None;
        };
        self.cels
            .get(path)?
            .get(usize::from(d))?
            .get(cel.index)
            .copied()
    }

    /// The `xoff`, `yoff` of [`Self::layer_cel`]: what gives
    /// a linked unit's offsets (component
    /// 14 S7). `None` is the (0, 0) case
    /// of that rule; the DT / DD test of the caller is not made here.
    pub fn cel_offset(
        &self,
        looks: &UnitLooks,
        unit: &ClientUnit,
        component: u8,
    ) -> Option<(i32, i32)> {
        self.layer_cel(looks, unit, component).map(|c| (c.ox, c.oy))
    }
}

/// [`UnitArt::cel_offset`] as a closure over the shared art, for the
/// bridge's model inputs (the client-do and motion readers).
pub fn cel_offset_fn(
    looks: Arc<UnitLooks>,
    art: SharedUnitArt,
) -> impl Fn(&ClientUnit, u8) -> Option<(i32, i32)> + Send + Sync + 'static {
    move |unit, component| {
        art.read()
            .unwrap_or_else(|e| e.into_inner())
            .cel_offset(&looks, unit, component)
    }
}

impl UnitArtLoader {
    /// Makes the COF and component files of every unit in `world`
    /// resident. Preview (`// D2RUST's own, unverified`): a file that fails is
    /// skipped and remembered; returns one log line per new failure.
    pub fn ensure(&self, world: &ClientWorld, assets: &mut ViewAssets) -> Vec<String> {
        let mut log = Vec::new();
        let mut art = self.art.write().unwrap_or_else(|e| e.into_inner());
        // The blocks the last drawn frame asked for are loaded.
        art.blocks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .settle(world.server_ticks);
        art.observe_facing(world);
        art.observe_equipment(world, &self.looks);
        art.gfx_ready = world.gfx_ready_now().cloned();
        for unit in world.view_units() {
            let unit = &*unit;
            let posed = art.posed(unit).into_owned();
            let unit = &*self.looks.shapes.identity(&posed);
            let Some(name) = unit_cof_with(&self.looks, unit, art.hands_of(unit.key)) else {
                continue;
            };
            let Ok(path) = name.path() else { continue };
            if art.missing_cofs.contains(&path) {
                continue;
            }
            if !art.cofs.contains_key(&path) {
                match self.read_cof(&name.full()) {
                    Ok(cof) => {
                        assets.cofs.insert(path.clone(), cof.clone());
                        art.cofs.insert(path.clone(), cof);
                    }
                    Err(e) => {
                        bevy::log::warn!(target: "d2_asset_fallback", asset = %name.full(), "{e}");
                        log.push(format!("unit art: {}: {e}", name.full()));
                        art.missing_cofs.insert(path);
                        continue;
                    }
                }
            }
            let cof = art.cofs[&path].clone();
            for layer in &cof.layers {
                let Some(codes) =
                    component_codes(&self.looks, unit, &name, layer, art.equip_of(unit.key))
                else {
                    continue;
                };
                let key = codes.name();
                if art.files.contains_key(&key) {
                    continue;
                }
                let format = file_format(&codes, unit.class, unit.mode as u8);
                let loaded = self.load_file(&codes, format, assets);
                // A file in no archive is the normal empty slot.
                if let Err(Some(e)) = &loaded {
                    bevy::log::warn!(target: "d2_asset_fallback", asset = %codes.file(format), "{e}");
                    log.push(format!("unit art: {}: {e}", codes.file(format)));
                }
                let loaded = loaded.ok().map(|(path, facts, cels)| {
                    art.cels.insert(path.clone(), cels);
                    (path, facts)
                });
                art.files.insert(key, loaded);
            }
        }
        log
    }

    /// The COF at `file`, typed: the native
    /// folder's file or the archives' decoded bytes.
    fn read_cof(&self, file: &str) -> Result<Cof, String> {
        crate::assets::path::read_cof(self.source.as_ref(), file)
            .ok_or_else(|| "in no archive".to_string())?
    }

    /// Every direction of the file into the frame store, all or nothing.
    /// `Err(None)`: the file is in no archive (not an error).
    fn load_file(
        &self,
        codes: &ComponentCodes,
        format: FileFormat,
        assets: &mut ViewAssets,
    ) -> Result<StoredFile, Option<String>> {
        let file = codes.file(format);
        let src = self.source.as_ref();
        let loaded = match format {
            FileFormat::Dcc => {
                crate::assets::path::read_dcc(src, &file).map(|r| r.map(Loaded::Dcc))
            }
            FileFormat::Dc6 => {
                crate::assets::path::read_dc6(src, &file).map(|r| r.map(Loaded::Dc6))
            }
        };
        let loaded = match loaded {
            None => return Err(None),
            Some(r) => r.map_err(Some)?,
        };
        self.store_file(codes, format, loaded, assets).map_err(Some)
    }

    fn store_file(
        &self,
        codes: &ComponentCodes,
        format: FileFormat,
        loaded: Loaded,
        assets: &mut ViewAssets,
    ) -> Result<StoredFile, String> {
        let path = codes.path(format).map_err(|e| e.to_string())?;
        let mut sets = Vec::new();
        let cels = cel_boxes(&loaded);
        let (directions, frames) = match format {
            FileFormat::Dcc => {
                let Loaded::Dcc(dcc) = loaded else {
                    return Err("format mismatch".into());
                };
                let d = u8::try_from(dcc.directions.len()).map_err(|_| "too many directions")?;
                for dir in 0..d {
                    let set = FrameSet::from_dcc(&dcc, dir).map_err(|e| e.to_string())?;
                    sets.push(sprite_cache_limit(set));
                }
                (d, dcc.frames_per_direction as usize)
            }
            FileFormat::Dc6 => {
                let Loaded::Dc6(dc6) = loaded else {
                    return Err("format mismatch".into());
                };
                let d = u8::try_from(dc6.header.directions).map_err(|_| "too many directions")?;
                for dir in 0..d {
                    sets.push(FrameSet::from_dc6(&dc6, dir).map_err(|e| e.to_string())?);
                }
                (d, dc6.header.frames_per_direction as usize)
            }
        };
        for (dir, set) in sets.into_iter().enumerate() {
            let key = FrameSetKey::new(path.as_str(), FramePart::Dir(dir as u8))
                .map_err(|e| e.to_string())?;
            if !assets.frames.contains(&key) {
                // The derived shadow set (`unit_shadow`), frame for frame.
                let shadow = super::unit_shadow::shadow_key(&key)?;
                let shadows = super::unit_shadow::shadow_set(&set);
                assets.frames.insert(key, set).map_err(|e| e.to_string())?;
                assets
                    .frames
                    .insert(shadow, shadows)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok((
            path,
            FileFacts {
                format,
                directions,
                frames,
            },
            cels,
        ))
    }
}

/// Largest DCC frame side the sprite cache draws
/// (the frame size limit).
pub const SPRITE_CACHE_SIDE: u32 = 256;

/// A component DCC direction as the sprite cache gives it:
/// a frame wider or taller than
/// [`SPRITE_CACHE_SIDE`] has no cel, so it draws nothing; the frame stays
/// in the set as an empty image, keeping every other index. No base-game draw
/// reaches such a frame: the only over-size files, `GTTRLITA1HTH.dcc` and
/// `GTTRLITNUHTH.dcc`, are named by no COF layer (the gargoyle trap's
/// `GTA1HTH.cof` / `GTNUHTH.cof` layers carry the weapon classes `att` /
/// `eut`), and a recorded draw of the trap matches: it draws the trap
/// from `GTTRlitNUeut.dcc` (52 × 94) on both sides.
pub fn sprite_cache_limit(mut set: FrameSet) -> FrameSet {
    for f in &mut set.frames {
        if f.width > SPRITE_CACHE_SIDE || f.height > SPRITE_CACHE_SIDE {
            f.width = 0;
            f.height = 0;
            f.pixels = Vec::new();
        }
    }
    set
}

/// The cel fields w, h, xoff, yoff of every frame:
/// a DC6 frame header as stored; a DCC frame header's width, height
/// and x / y offset, unchanged.
fn cel_boxes(loaded: &Loaded) -> Vec<Vec<CelBox>> {
    let i = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
    match loaded {
        Loaded::Dcc(dcc) => dcc
            .directions
            .iter()
            .map(|d| {
                d.frames
                    .iter()
                    .map(|f| CelBox {
                        w: i(f.width),
                        h: i(f.height),
                        ox: f.x_offset,
                        oy: f.y_offset,
                    })
                    .collect()
            })
            .collect(),
        Loaded::Dc6(dc6) => (0..dc6.header.directions as usize)
            .map(|d| {
                (0..dc6.header.frames_per_direction as usize)
                    .map_while(|f| dc6.frame(d, f))
                    .map(|f| CelBox {
                        w: i(f.width),
                        h: i(f.height),
                        ox: f.offset_x,
                        oy: f.offset_y,
                    })
                    .collect()
            })
            .collect(),
    }
}

/// A stored component file: its path, facts and cel fields by direction
/// and frame.
type StoredFile = (CanonicalPath, FileFacts, Vec<Vec<CelBox>>);

/// A unit art file read through [`FileSource::read_native`].
enum Loaded {
    Dcc(d2_formats::dcc::Dcc),
    Dc6(d2_formats::dc6::Dc6),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::IndexFrame;

    fn frame(w: u32, h: u32) -> IndexFrame {
        IndexFrame::new(w, h, -4, -7, vec![1; (w * h) as usize]).unwrap()
    }

    // A DCC frame with w > 256 or h > 256
    // draws nothing; frames up to 256 × 256 draw unchanged (no base-game draw
    // reaches an over-size frame, see `sprite_cache_limit`).
    #[test]
    fn sprite_cache_drops_frames_over_256() {
        let set = FrameSet {
            frames: vec![frame(256, 256), frame(257, 1), frame(1, 257), frame(3, 2)],
        };
        let out = sprite_cache_limit(set.clone());
        assert_eq!(out.frames[0], set.frames[0]);
        assert!(out.frames[1].is_empty() && out.frames[1].pixels.is_empty());
        assert!(out.frames[2].is_empty() && out.frames[2].pixels.is_empty());
        assert_eq!(out.frames[3], set.frames[3]);
    }

    #[test]
    fn an_unready_fallen_draws_with_four_directions() {
        // Measured on a War Cry cast: class 19 (Fallen, d<mode>
        // 8) draws dir 59 as 56 until its NU flag reads 1 at tick 183.
        let mut looks = UnitLooks::default();
        let mut directions = [8; 16];
        directions[3] = 16;
        looks.monsters.insert(
            19,
            MonsterRow {
                token: code(b"FA"),
                base_w: None,
                composite_death: false,
                directions,
            },
        );
        let mut unit = ClientUnit::new(UnitKey::new(1, 7));
        unit.class = 19;
        unit.mode = 1;
        let mut art = UnitArt::default();
        let n = |art: &UnitArt, unit: &ClientUnit| {
            art.expected_directions(&looks, unit, CompositeKind::Monster, 8)
        };
        // Not recorded: drawn as ready.
        assert_eq!(n(&art, &unit), 8);
        let mut ready = crate::bridge::world::GfxReady::default();
        ready.masks.insert(149, 0x106);
        art.gfx_ready = Some(ready.clone());
        assert_eq!(n(&art, &unit), 4);
        // d<mode> ≠ 8 (mode 3 here): the COF count.
        unit.mode = 3;
        assert_eq!(n(&art, &unit), 8);
        unit.mode = 1;
        ready.masks.insert(19, 0x337f);
        art.gfx_ready = Some(ready);
        assert_eq!(n(&art, &unit), 8);
    }

    // The turn step table and the stepper.
    #[test]
    fn a_cast_face_is_drawn_turning_by_steps() {
        let t = d2_sim::path::tables::PathTables::spec().unwrap();
        let mut want = vec![0];
        want.extend([1; 8]);
        want.extend([4; 8]);
        want.extend([8; 15]);
        want.extend([-8; 16]);
        want.extend([-4; 8]);
        want.extend([-1; 8]);
        assert_eq!(t.dirdiff, want, "the unit frame step table");

        let key = UnitKey::new(PLAYER, 3);
        let mut w = ClientWorld::default();
        let mut u = ClientUnit::new(key);
        u.position = Some((50, 50));
        u.path_dir = Some(0);
        w.units.insert(key, u);
        let mut art = UnitArt::default();
        art.observe_facing(&w);
        assert_eq!(art.dir64(&w.units[&key]), 0);
        // The cast faces direction 54 (flag 0): step −4, and
        // each update turns once, the last step ending on the target
        // (a Sorceress Fire Bolt cast: 0, 60, 56, 54).
        crate::bridge::critter_path::face_direction(&mut w, key, 54);
        let mut drawn = Vec::new();
        for _ in 0..4 {
            crate::bridge::critter_path::player_turn(&mut w, key);
            art.observe_facing(&w);
            drawn.push(art.dir64(&w.units[&key]));
        }
        assert_eq!(drawn, [60, 56, 54, 54]);
    }
}

#[cfg(test)]
mod player_look_tests {
    use super::*;

    fn look(gfx: &[u8], component: u8, type_class: u8, armor: Option<[u8; 6]>) -> ItemLook {
        ItemLook {
            gfx: code(gfx),
            component,
            type_class,
            armor,
            hand: hand_item(component, 0, *b"hth ", *b"hth ", 0),
            trans: (0, 0),
            weapon: matches!(type_class, 2 | 3 | 12),
            shield: false,
        }
    }

    #[test]
    fn a_held_sword_sets_the_cof_weapon_class() {
        let mut l = UnitLooks::default();
        let mut sword = look(b"ssd", 5, 2, None);
        sword.hand = hand_item(5, 30, *b"1hs ", *b"2hs ", 0);
        l.items.insert(code(b"ssd"), sword);
        assert_eq!(
            player_hands_class(&l, 4, &[(4, code(b"ssd"))], Some(4)),
            *b"1hs "
        );
        assert_eq!(player_hands_class(&l, 4, &[], None), HTH);
    }

    #[test]
    fn equipped_items_fill_their_components() {
        let mut l = UnitLooks::default();
        l.items.insert(code(b"cap"), look(b"cap", 0, 0, None));
        l.items
            .insert(code(b"qui"), look(b"qui", 1, 0, Some([1, 1, 1, 1, 0, 0])));
        l.items.insert(code(b"ssd"), look(b"ssd", 5, 2, None));
        l.items.insert(code(b"dgr"), look(b"dgr", 5, 2, None));
        let p = player_look(
            &l,
            &[
                (1, code(b"cap")),
                (3, code(b"qui")),
                (4, code(b"ssd")),
                (5, code(b"dgr")),
            ],
            Some(4),
        );
        assert_eq!(p.item_gfx[0], Some(code(b"cap")));
        assert_eq!(p.body_armor, Some([1, 1, 1, 1, 0, 0]));
        // 1hs items: the weapon in use is component 5, the other hand 6.
        assert_eq!(p.item_gfx[5], Some(code(b"ssd")));
        assert_eq!(p.item_gfx[6], Some(code(b"dgr")));
        assert_eq!(player_look(&l, &[], None).item_gfx, [None; 16]);
        // A weapon in use in the left hand (location 5) is component 5.
        let p = player_look(&l, &[(5, code(b"ssd"))], Some(5));
        assert_eq!((p.item_gfx[5], p.item_gfx[6]), (Some(code(b"ssd")), None));
    }
}

#[cfg(test)]
mod equipment_tests {
    use super::*;
    use crate::bridge::msg::support::Model;
    use crate::bridge::world::PlayerData;

    /// A recorded base-game 0x9D (Firelizard's Talons, `7lw`, at
    /// location 4; stream flags
    /// 0x00804011, requirement flag 0x4000 set).
    const TALONS_9D: &str = "9d 06 32 05 01 00 00 00 00 01 00 00 00 11 40 80 00 65 84 08 \
        70 c3 76 07 02 cf 01 17 00 11 c6 8d 3b 21 06 83 1d 7c 97 d1 1a 83 59 03 51 bc 64 00 f6 1f";

    fn hands(flag_word: &str) -> Option<Code> {
        let p = UnitKey::new(PLAYER, 1);
        let mut m = Model::default();
        m.put(p).kind = KindData::Player(PlayerData::default());
        m.w.local_player = Some(p);
        m.hex(&TALONS_9D.replacen("11 40 80 00", flag_word, 1));
        assert!(m.log.rejected.is_empty(), "{:?}", m.rejected());
        let mut l = UnitLooks::default();
        l.items.insert(
            code(b"7lw"),
            ItemLook {
                gfx: code(b"clw"),
                component: 5,
                type_class: 12,
                armor: None,
                hand: hand_item(5, 45, *b"ht1 ", *b"ht1 ", 0),
                trans: (0, 0),
                weapon: true,
                shield: false,
            },
        );
        let mut art = UnitArt::default();
        art.observe_equipment(&m.w, &l);
        art.hands_of(p)
    }

    #[test]
    fn a_hand_item_with_the_requirement_flag_is_not_held() {
        assert_eq!(hands("11 40 80 00"), None);
        assert_eq!(hands("11 00 80 00"), Some(*b"ht1 "));
    }
}

#[cfg(test)]
mod cel_offset_tests {
    //! Synthetic fixtures only: an invented object token, COF and DC6.
    use super::*;
    use crate::assets::path::MemorySource;
    use crate::bridge::world::OBJECT;
    use crate::world_view::ViewAssets;
    use d2_formats::palette::{Palette, Rgb};

    /// COF bytes: one direction, one frame, one layer of
    /// component 1, weapon class `hth`.
    fn cof() -> Vec<u8> {
        let mut v = vec![1, 1, 1, 20, 0, 0, 0, 0];
        for x in [-10i32, 10, -20, 0] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&256u32.to_le_bytes());
        v.extend_from_slice(&[1, 0, 1, 0, 0]);
        v.extend_from_slice(b"hth\0");
        v.push(0);
        v.push(1);
        v
    }

    /// A DC6 of one direction, one 4 × 2 frame at offset (−14, −91).
    fn dc6() -> Vec<u8> {
        let rows = [4u8, 1, 2, 3, 4, 0x80, 4, 1, 2, 3, 4, 0x80];
        let mut d = Vec::new();
        for v in [6i32, 1, 0] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&[0xEE; 4]);
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&((d.len() + 4) as u32).to_le_bytes());
        for v in [0i32, 4, 2, -14, -91, 0, 0, rows.len() as i32] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&rows);
        d.extend_from_slice(&[0xEE; 3]);
        d
    }

    #[test]
    fn the_layer_cel_offset_of_a_loaded_unit() {
        let mut src = MemorySource::default();
        src.insert("data\\global\\objects\\QO\\cof\\QOQNhth.cof", cof());
        src.insert("data\\global\\objects\\QO\\QT\\QOQTlitQNhth.dc6", dc6());
        let mut looks = UnitLooks {
            object_modes: vec![code(b"QN")],
            components: vec![code(b"QH"), code(b"QT")],
            ..Default::default()
        };
        looks.objects.insert(342, code(b"QO"));
        let looks = Arc::new(looks);
        let art = SharedUnitArt::default();
        let loader = UnitArtLoader {
            source: Arc::new(src),
            looks: looks.clone(),
            art: art.clone(),
        };
        let mut u = ClientUnit::new(UnitKey {
            unit_type: OBJECT,
            guid: 9,
        });
        u.class = 342;
        u.position = Some((10, 10));
        let mut world = ClientWorld::default();
        world.units.insert(u.key, u.clone());
        let mut assets = ViewAssets::new(Palette {
            colors: [Rgb::default(); 256],
        });
        loader.ensure(&world, &mut assets);
        let f = cel_offset_fn(looks, art);
        assert_eq!(f(&u, 1), Some((-14, -91)));
        // No layer of component 14 in the COF: the (0, 0) case.
        assert_eq!(f(&u, 14), None);
    }
}
