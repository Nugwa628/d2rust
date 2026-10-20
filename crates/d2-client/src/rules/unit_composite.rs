//! Unit composites: which COF and component
//! files a unit draws, the direction and frame indices, the component
//! request (armor class and weapon class), the colormap source per
//! component, the COF box pre-test, the per-unit extra offsets (client
//! motion record, object and missile offsets) and the single-cel units.
//! Plain Rust, integer math, no Bevy types.
//!
//! The slot loop itself (row offset, slots that draw nothing, S7) is
//! [`crate::composite`]; this module answers the questions that loop asks.
//! Inputs the client model does not hold yet (equipped items, monster
//! component choices, table rows) arrive as plain values from the caller.
//! A rule whose outcome is not known is a `TODO(spec: …)` that returns
//! [`UnitCompositeError::Unresolved`], never a guess.

use d2_formats::cof::Cof;

use crate::assets::CanonicalPath;
use crate::composite::ComponentFrame;
use crate::frames::{FramePart, FrameSetKey};
use crate::world_view::UnitPose;

const SPEC: &str = "render/unit-composite.md";

/// Errors of this module: the game's fatal checks, and cases whose outcome
/// is not known, which are refused rather than guessed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnitCompositeError {
    /// COF or file direction count not a power of two below 128 (fatal
    /// 0x1ED / 0x1EE).
    #[error("direction count {0} is not a power of two below 128")]
    DirectionCount(u8),
    /// A cel past `frame ≤ Ff` (fatal), or past the file's last cel.
    #[error(
        "cel of file direction {dir}, frame {frame} is outside the file ({directions} × {frames})"
    )]
    Cel {
        dir: u8,
        frame: usize,
        directions: u8,
        frames: usize,
    },
    /// An `armtype` index above 2: there is no fourth table entry to
    /// read. The request fails (the slot draws nothing) and D2RUST
    /// reports it; no armor row of the game's data has such a byte.
    #[error("armtype index {0} is past the 3-entry table")]
    ArmTypeIndex(u8),
    /// An act II skeleton choice past the reachable entries of its
    /// override array: there is no entry to read.
    #[error("act II override of component {component}, choice {choice} is past its table")]
    OverrideTable { component: u8, choice: u8 },
    /// A missile following a linked unit that is not a monster (fatal
    /// 0x1A9).
    #[error("missile motion record follows a unit that is not a monster")]
    FollowTarget,
    /// A composed path the asset layer refuses.
    #[error("path {0:?}: {1}")]
    Path(String, String),
    /// A rule whose outcome is not known (`what` names it).
    #[error("{what}: TODO(spec: {SPEC} {question})")]
    Unresolved {
        what: &'static str,
        question: &'static str,
    },
}

// ---------------------------------------------------------------------------
// Codes and names: 4-byte codes, and the name parts built from them.

/// A 4-byte code as the tables store it, space-padded (`lit `, `hth `).
pub type Code = [u8; 4];

pub const LIT: Code = *b"lit ";
pub const MED: Code = *b"med ";
pub const HVY: Code = *b"hvy ";
pub const HTH: Code = *b"hth ";
pub const HT2: Code = *b"ht2 ";
pub const HSH: Code = *b"hsh ";
pub const XBW: Code = *b"xbw ";
pub const BOW: Code = *b"bow ";
/// Mode token of an empty table entry.
pub const XXX: Code = *b"xxx ";
/// No code (an empty table field).
pub const EMPTY: Code = [0; 4];

/// The `armtype` tokens by index.
pub const ARMTYPE: [Code; 3] = [LIT, MED, HVY];

pub use d2_server::game::composite_names::{
    code, mode_overrides, mode_token, monster_weapon_class, part, CofName, CompositeKind,
    MONSTER_MODE_OVERRIDES, PLAYER_MODE_OVERRIDES, PLAYER_MODE_TH,
};
/// Client asset-path adaptation for a shared composite name.
pub trait CofPath {
    fn path(&self) -> Result<CanonicalPath, UnitCompositeError>;
}
impl CofPath for CofName {
    fn path(&self) -> Result<CanonicalPath, UnitCompositeError> {
        canonical(self.full())
    }
}
fn text(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}
fn canonical(path: String) -> Result<CanonicalPath, UnitCompositeError> {
    CanonicalPath::new(&path).map_err(|e| UnitCompositeError::Path(path, e.to_string()))
}

/// One hand item of a player as the weapon class reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandItem {
    /// The item's `component` (5 RH, 6 LH count).
    pub component: u8,
    /// Item type (45 is the type of the dual-wield test).
    pub item_type: u16,
    pub wclass: Code,
    pub two_handed_wclass: Code,
    /// The item's grip test says two-handed.
    pub two_handed_grip: bool,
}

/// A hand body location: 4 right hand, 5 left hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandLoc {
    Loc4,
    Loc5,
}

/// What the weapon class reads for a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerHands {
    /// `charstats` class (4 Barbarian, 6 Assassin).
    pub class: u8,
    /// `charstats` weapon class (+0x4C).
    pub base_wclass: Code,
    /// Valid items in body locations 4 and 5.
    pub loc4: Option<HandItem>,
    pub loc5: Option<HandItem>,
    /// The location of the weapon in use, read by the Barbarian
    /// dual-wield rule.
    pub in_use: Option<HandLoc>,
}

/// Item type class of a weapon class (a fixed table, used by the dual-wield
/// rule): `bow` 1, `1hs` 2, `1ht` 3, `stf` 4, `2hs` 5,
/// `2ht` 6, `xbw` 7, `ht1` 12, otherwise 0.
pub fn weapon_type_class(wclass: Code) -> u8 {
    const TABLE: [(&[u8], u8); 8] = [
        (b"bow", 1),
        (b"1hs", 2),
        (b"1ht", 3),
        (b"stf", 4),
        (b"2hs", 5),
        (b"2ht", 6),
        (b"xbw", 7),
        (b"ht1", 12),
    ];
    TABLE
        .iter()
        .find(|(w, _)| part(&wclass) == *w)
        .map_or(0, |&(_, c)| c)
}

/// The weapon class, with the draw-time dual-wield write: the class, and
/// `Some(HandLoc::Loc4)` when the Barbarian rule found no weapon in use
/// and made the right-hand item the weapon in use.
fn player_weapon_class_and_write(hands: &PlayerHands) -> (Code, Option<HandLoc>) {
    let in_hand = |i: &Option<HandItem>| i.filter(|i| i.component == 5 || i.component == 6);
    let Some(hand) = in_hand(&hands.loc4).or_else(|| in_hand(&hands.loc5)) else {
        return (hands.base_wclass, None);
    };
    if let (Some(r), Some(l)) = (hands.loc4, hands.loc5) {
        if r.item_type == 45 && l.item_type == 45 {
            match hands.class {
                4 => {
                    // r-dual: `A` the weapon in use (none: the right-hand
                    // item, written back), `B` the other hand item.
                    let (a, b, write) = match hands.in_use {
                        Some(HandLoc::Loc5) => (l, r, None),
                        Some(HandLoc::Loc4) => (r, l, None),
                        None => (r, l, Some(HandLoc::Loc4)),
                    };
                    let w = match (weapon_type_class(a.wclass), weapon_type_class(b.wclass)) {
                        (2, 3) => *b"1js ",
                        (3, 3) => *b"1jt ",
                        (3, 2) => *b"1st ",
                        _ => *b"1ss ",
                    };
                    return (w, write);
                }
                6 => return (HT2, None),
                _ => {}
            }
        }
    }
    let w = if hand.two_handed_grip {
        hand.two_handed_wclass
    } else {
        hand.wclass
    };
    (w, None)
}

/// The draw-time write of the dual-wield rule: `Some(HandLoc::Loc4)` when a
/// Barbarian holding two type-45 items has no weapon in use; the client
/// then makes the right-hand item the weapon in use.
pub fn weapon_in_use_write(hands: &PlayerHands) -> Option<HandLoc> {
    player_weapon_class_and_write(hands).1
}

/// Player weapon class for the component request.
/// For the COF name, modes DT/DD use `hth` instead
/// ([`player_cof_weapon_class`]).
/// The draw-time write of the Barbarian rule is [`weapon_in_use_write`].
pub fn player_weapon_class(hands: &PlayerHands) -> Result<Code, UnitCompositeError> {
    Ok(player_weapon_class_and_write(hands).0)
}

/// Player weapon class in the COF name: `hth` in
/// modes DT/DD, else [`player_weapon_class`].
pub fn player_cof_weapon_class(mode: u8, hands: &PlayerHands) -> Result<Code, UnitCompositeError> {
    if CompositeKind::Player.is_death_mode(mode) {
        return Ok(HTH);
    }
    player_weapon_class(hands)
}

// ---------------------------------------------------------------------------
// Direction and frame; tables of `unit-directions.tsv`.

/// `log2(d)` for a power-of-two direction count below 128.
fn log2(d: u8) -> Result<u32, UnitCompositeError> {
    if d == 0 || !d.is_power_of_two() || d >= 128 {
        return Err(UnitCompositeError::DirectionCount(d));
    }
    Ok(d.trailing_zeros())
}

/// COF direction of `dir64` for a COF of `d` directions:
/// `((dir64 + 32/D) >> log2(64/D)) mod D`. `dir64` ≥ 64 is
/// direction 0.
pub fn cof_direction(d: u8, dir64: u8) -> Result<u8, UnitCompositeError> {
    let k = log2(d)?;
    if dir64 >= 64 {
        return Ok(0);
    }
    // k ≤ 6: `log2` refuses 128.
    let shift = 6 - k;
    let half = 32u32 >> k;
    Ok((((u32::from(dir64) + half) >> shift) % u32::from(d)) as u8)
}

/// `P_D`: the file direction stored at angular step `k`
/// for a file of `d` directions. `P_1 = [0]`, `P_2 = [0, 0]`,
/// `P_4 = [0, 1, 2, 3]`, `P_8[2j] = 4 + j`, `P_8[2j+1] = j`,
/// `P_2D[2j] = P_D[j]`, `P_2D[2j+1] = D + j`.
fn interleave(d: u8, k: u8) -> u8 {
    match d {
        1 | 2 => 0,
        4 => k,
        8 => {
            if k.is_multiple_of(2) {
                4 + k / 2
            } else {
                k / 2
            }
        }
        _ => {
            let half = d / 2;
            if k.is_multiple_of(2) {
                interleave(half, k / 2)
            } else {
                half + k / 2
            }
        }
    }
}

/// File direction of `dir64` for a DCC/DC6 of `df` directions:
/// `P_Df[cof_direction(Df, dir64)]`.
pub fn file_direction(df: u8, dir64: u8) -> Result<u8, UnitCompositeError> {
    let k = cof_direction(df, dir64)?;
    Ok(interleave(df, k))
}

/// Snap tables of (8, 4) and (16, 8).
const SNAP_8_4: [u8; 8] = [1, 1, 3, 3, 5, 5, 7, 7];
const SNAP_16_8: [u8; 16] = [0, 0, 2, 2, 4, 4, 6, 6, 8, 8, 10, 10, 12, 12, 14, 14];

/// `dir64` snapped for (COF directions `d`, expected count `n`): only
/// (8, 4) and (16, 8) snap.
pub fn snap(d: u8, n: u8, dir64: u8) -> u8 {
    match (d, n) {
        (8, 4) => SNAP_8_4[usize::from(dir64 >> 3) & 7] << 3,
        (16, 8) => SNAP_16_8[usize::from(dir64 >> 2) & 15] << 2,
        _ => dir64,
    }
}

/// Who a unit's expected direction count comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionSource {
    /// `sixteen`: the local player while the sixteen-direction switch is
    /// off (the caller decides when that is).
    Player {
        sixteen: bool,
    },
    /// monstats2 `d<mode>` (+0xF4 + mode); `mode_table_zero`: the class's
    /// graphics-ready flag for the mode is 0. With
    /// synchronous loading D2RUST treats a loaded class and mode as ready.
    Monster {
        d_mode: u8,
        mode_table_zero: bool,
    },
    /// `missiles` NumDirections.
    Missile {
        num_directions: u8,
    },
    Other,
}

/// Expected direction count `n`. A mode < 0 gives 1.
pub fn expected_directions(source: DirectionSource, mode: i32) -> u8 {
    if mode < 0 {
        return 1;
    }
    match source {
        DirectionSource::Player { sixteen } => {
            if sixteen {
                16
            } else {
                8
            }
        }
        DirectionSource::Monster {
            d_mode,
            mode_table_zero,
        } => {
            if d_mode == 8 && mode_table_zero {
                4
            } else {
                d_mode
            }
        }
        DirectionSource::Missile { num_directions } => num_directions,
        DirectionSource::Other => 1,
    }
}

/// The direction answers of one drawn composite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitDirection {
    /// `dir64` after the snap; component files use this.
    pub dir64: u8,
    /// The COF row direction.
    pub cof_dir: u8,
    /// The value written back into the unit's path during the draw
    /// (dead player or monster with `n` ≠ `D`), else `None`.
    pub write_back: Option<u8>,
}

/// The direction answers for a unit facing `dir64` with expected count `n`, a COF of
/// `d` directions, `dead` = mode DT/DD of a player or monster.
pub fn unit_direction(
    d: u8,
    n: u8,
    dir64: u8,
    dead: bool,
) -> Result<UnitDirection, UnitCompositeError> {
    log2(d)?;
    // dir64 ≥ 64 is direction 0 with n = 1.
    let (dir64, n) = if dir64 >= 64 { (0, 1) } else { (dir64, n) };
    let snapped = snap(d, n, dir64);
    Ok(UnitDirection {
        dir64: snapped,
        cof_dir: cof_direction(d, snapped)?,
        write_back: (n != d && dead).then_some(snapped),
    })
}

/// Where a unit's `dir64` comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirSource {
    /// Dynamic path direction: types 0, 1, 3.
    DynamicPath,
    /// Static path +0x1C byte: types 2, 4.
    StaticPathByte,
}

/// The `dir64` source by unit type.
pub fn dir_source(unit_type: u32) -> Option<DirSource> {
    match unit_type {
        0 | 1 | 3 => Some(DirSource::DynamicPath),
        2 | 4 => Some(DirSource::StaticPathByte),
        _ => None,
    }
}

/// The frame index of a unit frame counter (8.8 fixed point).
pub fn frame_index(frame_counter: u32) -> usize {
    (frame_counter >> 8) as usize
}

/// The cel of a component file: file direction from `dir64` (the
/// snapped value), cel `Ff × dcc_dir + frame` as a frame set of that file
/// (one set per file direction, `FramePart::Dir`). `frame = Ff` passes
/// the frame check and is the next direction's first cel; past the
/// last cel is an error (there is no cel to read).
pub fn component_cel(
    path: &CanonicalPath,
    df: u8,
    ff: usize,
    dir64: u8,
    frame: usize,
) -> Result<ComponentFrame, UnitCompositeError> {
    let dcc_dir = file_direction(df, dir64)?;
    let bad = || UnitCompositeError::Cel {
        dir: dcc_dir,
        frame,
        directions: df,
        frames: ff,
    };
    if frame > ff || ff == 0 {
        return Err(bad());
    }
    let cel = ff * usize::from(dcc_dir) + frame;
    let dir = u8::try_from(cel / ff)
        .ok()
        .filter(|&d| d < df)
        .ok_or_else(bad)?;
    let set = FrameSetKey::new(path.as_str(), FramePart::Dir(dir))
        .map_err(|e| UnitCompositeError::Path(path.to_string(), e.to_string()))?;
    Ok(ComponentFrame {
        set,
        index: cel % ff,
    })
}

/// `ComponentResolver::slot_frame`: `None` when the
/// request failed (`codes` = `None`) or the file did not load (`file` =
/// `None`), else the cel of [`component_cel`]. `file` is the loaded
/// file's (`Df`, `Ff`).
pub fn slot_frame(
    codes: Option<&ComponentCodes>,
    format: FileFormat,
    file: Option<(u8, usize)>,
    dir64: u8,
    frame: usize,
) -> Result<Option<ComponentFrame>, UnitCompositeError> {
    let (Some(codes), Some((df, ff))) = (codes, file) else {
        return Ok(None);
    };
    component_cel(&codes.path(format)?, df, ff, dir64, frame).map(Some)
}

/// `ViewRules::unit_pose`: `None` unless the unit is of type 0–2
/// (`kind`), its COF loaded (`cof`; no COF: not drawn) and the COF
/// box passes the box culling at the final screen position `at` in a `w`
/// × `h` frame. `n` is the expected direction count, `dead` the DT/DD
/// test of the write-back. Returns
/// (snapped `dir64` for the component files, write-back).
#[allow(clippy::too_many_arguments)]
pub fn unit_pose(
    kind: Option<CompositeKind>,
    name: &CofName,
    cof: Option<&Cof>,
    at: (i32, i32),
    size: (u32, u32),
    n: u8,
    dir64: u8,
    dead: bool,
    frame: usize,
) -> Result<Option<(UnitPose, UnitDirection)>, UnitCompositeError> {
    let (Some(_), Some(cof)) = (kind, cof) else {
        return Ok(None);
    };
    if !cof_box_visible(cof, at.0, at.1, size.0, size.1) {
        return Ok(None);
    }
    let direction = unit_direction(cof.directions, n, dir64, dead)?;
    Ok(Some((
        UnitPose {
            sub_frame: 0,
            cof: name.path()?,
            dir: usize::from(direction.cof_dir),
            frame,
            dir64: direction.dir64,
        },
        direction,
    )))
}

// ---------------------------------------------------------------------------
// COF box culling.

/// The box culling: the composite at final screen position (`x`, `y`)
/// in a `w` × `h` frame is drawn only if the COF box passes all four
/// tests.
pub fn cof_box_visible(cof: &Cof, x: i32, y: i32, w: u32, h: u32) -> bool {
    let (x, y) = (i64::from(x), i64::from(y));
    let (w, h) = (i64::from(w), i64::from(h));
    i64::from(cof.x_min) + x < w - 1
        && i64::from(cof.x_max) + x >= 0
        && i64::from(cof.y_max) + y >= 0
        && i64::from(cof.y_min) + y < h - 1
}

/// The shadow pre-test at the shadow position (`x`, `y`): the box
/// culling with the left bound moved by
/// `d = (y_max − y_min) / 2` (C division); the y bounds are the whole
/// COF box.
pub fn shadow_box_visible(cof: &Cof, x: i32, y: i32, w: u32, h: u32) -> bool {
    let (x, y) = (i64::from(x), i64::from(y));
    let (w, h) = (i64::from(w), i64::from(h));
    let d = (i64::from(cof.y_max) - i64::from(cof.y_min)) / 2;
    i64::from(cof.x_min) - d + x < w - 1
        && i64::from(cof.x_max) + x >= 0
        && i64::from(cof.y_max) + y >= 0
        && i64::from(cof.y_min) + y < h - 1
}

// ---------------------------------------------------------------------------
// Component request and file.

/// Component IDs used by the rules (`COMPONENT_NAMES` in `d2-formats`).
pub mod component {
    pub const HD: u8 = 0;
    pub const TR: u8 = 1;
    pub const LG: u8 = 2;
    pub const RA: u8 = 3;
    pub const LA: u8 = 4;
    pub const RH: u8 = 5;
    pub const LH: u8 = 6;
    pub const SH: u8 = 7;
    pub const S1: u8 = 8;
    pub const S2: u8 = 9;
    pub const S7: u8 = 14;
    pub const S8: u8 = 15;
}

/// The five codes of a component request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentCodes {
    pub kind: CompositeKind,
    pub unit_token: Code,
    pub component_token: Code,
    pub armor_class: Code,
    pub mode_token: Code,
    pub weapon_class: Code,
}

/// The armor-class source of a request, per unit kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmorSource<'a> {
    /// Objects: `lit`.
    Object,
    Monster(&'a MonsterLook),
    /// `None`: the player has no inventory (the request fails).
    Player(Option<&'a PlayerLook>),
}

/// What the component request reads for a monster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterLook {
    /// Client monster data +4: the choice per component.
    pub choices: [u8; 16],
    /// monstats2 counts per component (+0x15).
    pub counts: [u8; 16],
    /// `compcode` code of each component's choice bytes (+0x26 + 12c + v),
    /// already looked up by the caller: `codes[c][v]`.
    pub codes: [[Code; 12]; 16],
    /// monstats2 `compositeDeath`.
    pub composite_death: bool,
    /// The unit's room is in act II.
    pub act_two: bool,
    /// The monster's base class (`monstats` `BaseId`).
    pub base_class: u32,
}

/// What the component request reads for a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerLook {
    /// Valid body armor (location 3): `torso`, `legs`, `rArm`, `lArm`,
    /// `rspad`, `lspad` bytes (+0x116 … +0x11B).
    pub body_armor: Option<[u8; 6]>,
    /// Gfx code (`alternategfx`, else `code`) of the valid equipped item
    /// for each component (for 5 the primary weapon, for 6 the other
    /// hand item).
    pub item_gfx: [Option<Code>; 16],
    /// State 101 `holyshield`.
    pub holy_shield: bool,
    /// An item in the shield hand.
    pub shield_hand_item: bool,
}

impl PlayerLook {
    /// The linked-unit inventory for the item components (all but TR, LG,
    /// RA, LA, S1, S2): the item codes and the
    /// shield-hand item come from `linked`'s inventory; the body armor and
    /// the `holyshield` state stay the unit's own. Use it when
    /// [`linked_inventory`] holds.
    pub fn with_linked_items(&self, linked: &PlayerLook) -> PlayerLook {
        PlayerLook {
            item_gfx: linked.item_gfx,
            shield_hand_item: linked.shield_hand_item,
            ..*self
        }
    }
}

/// A draw reads the linked unit's inventory instead of the unit's when
/// flag-ex bit 3 is set, the unit has a state with `states` flag
/// `bossinv` and the linked unit exists.
pub fn linked_inventory(flag_ex_bit3: bool, bossinv_state: bool, linked_exists: bool) -> bool {
    flag_ex_bit3 && bossinv_state && linked_exists
}

/// Index into `torso` … `lspad` of the armor-class components.
fn armor_slot(c: u8) -> Option<usize> {
    use component::*;
    match c {
        TR => Some(0),
        LG => Some(1),
        RA => Some(2),
        LA => Some(3),
        S1 => Some(4),
        S2 => Some(5),
        _ => None,
    }
}

/// The armor class of component `c`, or `None` when the
/// request fails (the slot draws nothing). `mode` is the unit's mode and
/// `weapon_class` the request's.
pub fn armor_class(
    source: ArmorSource<'_>,
    c: u8,
    mode: u8,
    weapon_class: Code,
) -> Result<Option<Code>, UnitCompositeError> {
    use component::*;
    let c16 = usize::from(c & 15);
    match source {
        ArmorSource::Object => Ok(Some(LIT)),
        ArmorSource::Monster(m) => {
            if CompositeKind::Monster.is_death_mode(mode) && !m.composite_death {
                return Ok(Some(LIT));
            }
            let v = m.choices[c16];
            if v >= m.counts[c16] || usize::from(v) >= 12 {
                return Ok(Some(LIT));
            }
            let looked_up = m.codes[c16][usize::from(v)];
            // The act II override: after the lookup, only for components with a table.
            let table = if m.act_two {
                act_two_table(m.base_class, c)
            } else {
                None
            };
            let Some(table) = table else {
                return Ok(Some(looked_up));
            };
            let code = table
                .get(usize::from(v))
                .ok_or(UnitCompositeError::OverrideTable {
                    component: c,
                    choice: v,
                })?;
            // A zero code makes the request fail.
            Ok(*code)
        }
        ArmorSource::Player(None) => Ok(None),
        ArmorSource::Player(Some(p)) => {
            if let Some(i) = armor_slot(c) {
                if CompositeKind::Player.is_death_mode(mode) {
                    return Ok(Some(LIT));
                }
                let index = p.body_armor.map_or(0, |b| b[i]);
                // An index above 2 is past `armtype`:
                // the request fails and is reported.
                return ARMTYPE
                    .get(usize::from(index))
                    .copied()
                    .map(Some)
                    .ok_or(UnitCompositeError::ArmTypeIndex(index));
            }
            let gfx = |c: u8| p.item_gfx[usize::from(c)].unwrap_or(LIT);
            Ok(Some(match c {
                RH | LH if part(&weapon_class) == b"xbw" => gfx(RH),
                RH if part(&weapon_class) == b"bow" => LIT,
                SH if p.holy_shield && p.shield_hand_item => HSH,
                _ => gfx(c),
            }))
        }
    }
}

/// `des `: the act II skeleton variant.
pub const DES: Code = *b"des ";

/// `monstats` base classes with act II override tables.
pub const BASE_SKELETON1: u32 = 0;
pub const BASE_SK_ARCHER1: u32 = 170;

const fn c4(s: &[u8; 3]) -> Option<Code> {
    Some([s[0], s[1], s[2], b' '])
}
const LIT3: Option<Code> = Some(LIT);
const DES3: Option<Code> = Some(DES);
const MED3: Option<Code> = Some(MED);
const HVY3: Option<Code> = Some(HVY);
/// A zero code (the request fails).
const ZERO: Option<Code> = None;

const SK_HD: [Option<Code>; 7] = [LIT3, LIT3, DES3, DES3, HVY3, HVY3, HVY3];
const SK_TR: [Option<Code>; 3] = [LIT3, MED3, HVY3];
const SK_LIMB: [Option<Code>; 3] = [LIT3, DES3, HVY3];
const SK_RH: [Option<Code>; 10] = [
    c4(b"axe"),
    c4(b"axe"),
    c4(b"fla"),
    c4(b"fla"),
    c4(b"hax"),
    c4(b"hax"),
    c4(b"mac"),
    c4(b"mac"),
    c4(b"scm"),
    c4(b"scm"),
];
const SK_SH: [Option<Code>; 5] = [ZERO, c4(b"buc"), c4(b"lrg"), c4(b"kit"), c4(b"sml")];
const SK_SPAD: [Option<Code>; 12] = [
    ZERO, ZERO, ZERO, ZERO, ZERO, ZERO, ZERO, ZERO, ZERO, LIT3, DES3, HVY3,
];
const SA_LH: [Option<Code>; 1] = [c4(b"sbw")];

/// The act II override array of (`base`, component `c`), `None` for no
/// table. Only the entries reachable with the game's `monstats2` counts
/// are held; an
/// entry `None` is a zero code. `sk_archer1` S1 / S2 have a table with no
/// reachable entry.
pub fn act_two_table(base: u32, c: u8) -> Option<&'static [Option<Code>]> {
    use component::*;
    match (base, c) {
        (BASE_SKELETON1, HD) => Some(&SK_HD),
        (BASE_SKELETON1, TR) => Some(&SK_TR),
        (BASE_SKELETON1, LG | RA | LA) => Some(&SK_LIMB),
        (BASE_SKELETON1, RH) => Some(&SK_RH),
        (BASE_SKELETON1, SH) => Some(&SK_SH),
        (BASE_SKELETON1, S1 | S2) => Some(&SK_SPAD),
        (BASE_SK_ARCHER1, HD | LG | RA | LA) => Some(&SK_LIMB),
        (BASE_SK_ARCHER1, TR) => Some(&SK_TR),
        (BASE_SK_ARCHER1, LH) => Some(&SA_LH),
        (BASE_SK_ARCHER1, S1 | S2) => Some(&[]),
        _ => None,
    }
}

impl ComponentCodes {
    /// The component request, or `None` when it fails (empty unit token or
    /// component token; armor class 0). An empty mode token becomes
    /// `xxx `; an empty weapon class (a COF layer's) does not
    /// fail: the name has no weapon-class part.
    pub fn new(
        kind: CompositeKind,
        unit_token: Code,
        component_token: Code,
        armor_class: Option<Code>,
        mode_token: Code,
        weapon_class: Code,
    ) -> Option<ComponentCodes> {
        let armor_class = armor_class.filter(|a| *a != EMPTY)?;
        if part(&unit_token).is_empty() || part(&component_token).is_empty() {
            return None;
        }
        let mode_token = if part(&mode_token).is_empty() {
            XXX
        } else {
            mode_token
        };
        Some(ComponentCodes {
            kind,
            unit_token,
            component_token,
            armor_class,
            mode_token,
            weapon_class,
        })
    }

    /// File name: the five parts.
    pub fn name(&self) -> String {
        [
            &self.unit_token,
            &self.component_token,
            &self.armor_class,
            &self.mode_token,
            &self.weapon_class,
        ]
        .iter()
        .map(|c| text(part(c)))
        .collect()
    }

    /// Path: `<root>\<T>\<C>\<name>.<ext>`.
    pub fn file(&self, format: FileFormat) -> String {
        format!(
            "{}\\{}\\{}\\{}.{}",
            self.kind.root(),
            text(part(&self.unit_token)),
            text(part(&self.component_token)),
            self.name(),
            format.extension()
        )
    }

    /// [`ComponentCodes::file`] as an asset path.
    pub fn path(&self, format: FileFormat) -> Result<CanonicalPath, UnitCompositeError> {
        canonical(self.file(format))
    }
}

/// Component file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFormat {
    Dcc,
    Dc6,
}

impl FileFormat {
    pub fn extension(self) -> &'static str {
        match self {
            FileFormat::Dcc => "dcc",
            FileFormat::Dc6 => "dc6",
        }
    }
}

/// Monsters always drawn from DC6 (`monstats` row indices).
pub const DC6_MONSTERS: [u32; 5] = [242, 251, 367, 521, 704];
/// Monsters drawn from DC6 in mode 0 (DT) only.
pub const DC6_MONSTERS_DEATH: [u32; 8] = [243, 284, 333, 544, 559, 570, 705, 709];
/// Objects always drawn from DC6 (`objects` row indices).
pub const DC6_OBJECTS: [u32; 2] = [342, 563];
/// The one file name drawn from DC6 by name.
pub const DC6_NAME: &str = "OYTRlitTNhth";

/// The component file with `CompressedData` = 1 (the only supported
/// value). `class` is the `monstats` / `objects` row index.
pub fn file_format(codes: &ComponentCodes, class: u32, mode: u8) -> FileFormat {
    let dc6 = match codes.kind {
        CompositeKind::Monster => {
            DC6_MONSTERS.contains(&class) || (mode == 0 && DC6_MONSTERS_DEATH.contains(&class))
        }
        CompositeKind::Object => DC6_OBJECTS.contains(&class),
        CompositeKind::Player => false,
    };
    // ASCII case-insensitive over the whole name.
    if dc6 || codes.name().eq_ignore_ascii_case(DC6_NAME) {
        FileFormat::Dc6
    } else {
        FileFormat::Dcc
    }
}

// ---------------------------------------------------------------------------
// Colormap source.

/// The unit map `U`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitMap {
    None,
    /// The unit's shift table row.
    ShiftRow(u8),
    /// The monster's palette shift (`palshift.dat`).
    MonsterPalShift,
}

/// The unit's map: palette index ≠ 0 → shift row; else monsters' palette shift;
/// else none.
pub fn unit_map(palette_index: u8, kind: CompositeKind) -> UnitMap {
    if palette_index != 0 {
        UnitMap::ShiftRow(palette_index)
    } else if kind == CompositeKind::Monster {
        UnitMap::MonsterPalShift
    } else {
        UnitMap::None
    }
}

/// The colormap source of one slot. The maps themselves belong to the
/// shading rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColormapSource {
    None,
    Unit(UnitMap),
    /// The item colour of the equipped item of `component`, `fallback`
    /// when that gives none.
    Item {
        component: u8,
        fallback: UnitMap,
    },
    /// S8 local blood.
    Blood,
}

/// One equipped item as the colormap source reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemLook {
    pub flags: u32,
    /// Gfx code.
    pub gfx: Code,
}

/// What the colormap source reads beyond the component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColormapInputs<'a> {
    pub kind: CompositeKind,
    pub unit: UnitMap,
    /// Unit +0x6C.
    pub palette_index: u8,
    /// The item-map condition (the unit's item maps are on, or it is the
    /// local player with item maps shown), evaluated by the caller.
    pub item_maps: bool,
    /// Equipped items by component, players only.
    pub items: &'a [Option<ItemLook>; 16],
    /// monstats2 `localBlood` ≠ 0 and local blood on.
    pub local_blood: bool,
}

/// The colormap source of component `c`.
pub fn colormap_source(c: u8, inputs: &ColormapInputs<'_>) -> ColormapSource {
    use component::*;
    match c {
        S8 => {
            if inputs.kind == CompositeKind::Monster && inputs.local_blood {
                ColormapSource::Blood
            } else {
                ColormapSource::None
            }
        }
        S7 => ColormapSource::None,
        _ => {
            let player = inputs.kind == CompositeKind::Player;
            if !player || !inputs.item_maps || inputs.palette_index == 0x6C {
                return ColormapSource::Unit(inputs.unit);
            }
            let own = inputs.items.get(usize::from(c)).copied().flatten();
            let (component, item) = match own {
                Some(i) => (c, Some(i)),
                None if matches!(c, LG | RA | LA | S1 | S2) => (TR, inputs.items[usize::from(TR)]),
                None => (c, None),
            };
            match item {
                Some(i) if i.flags & (0x100 | 0x4000) == 0 => {
                    if part(&i.gfx) == b"lit" {
                        ColormapSource::None
                    } else {
                        ColormapSource::Item {
                            component,
                            fallback: inputs.unit,
                        }
                    }
                }
                _ => ColormapSource::Unit(inputs.unit),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Extra offsets: the motion record.

/// Client motion record flags.
pub mod motion {
    pub const DONE: i32 = 1;
    pub const TIMED: i32 = 2;
    pub const BOUNCE: i32 = 4;
    pub const RESTARTED: i32 = 8;
    pub const FOLLOW: i32 = 0x10;
    pub const FROM_BELOW: i32 = 0x20;
}

/// The 0x4C-byte client motion record. Positions are 16.16 subtile
/// (`>> 11` is 1/32 subtile), limits 1/32 subtile, offsets pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MotionRecord {
    pub flags: i32,
    pub pos: [i32; 3],
    pub vel: [i32; 3],
    pub acc: [i32; 3],
    pub limit: [i32; 3],
    /// `ox`, `oy`, `oz` (read by the draw).
    pub offset: [i32; 3],
    pub bounces_left: i32,
    /// Percent.
    pub bounce_factor: i32,
    pub ticks_left: i32,
}

/// The linked unit `K` a record with flag 0x10 follows, as the
/// caller reads it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FollowTarget {
    /// `K` is a monster (type 1).
    pub monster: bool,
    /// `K`'s mode (DT 0 / DD 12 stop a following missile).
    pub mode: u8,
    /// (`a`, `b`): the `xoff`, `yoff` of `K`'s component 14 (S7) cel for
    /// its current frame and direction, or (0, 0) when `K`'s
    /// COF for its draw mode is not loaded, its loaded-COF field `+0x14` is
    /// 0, or the request fails.
    pub s7_offset: (i32, i32),
    /// `K`'s own `ox`, `oy`, `oz` (its motion record; 0 without one).
    pub offset: [i32; 3],
}

impl MotionRecord {
    /// One update, once per client unit update;
    /// nothing when done. `missile`: this unit is a missile; `linked`: the
    /// linked unit `K` of the follow branch, `None` when it
    /// does not exist.
    pub fn update(
        &mut self,
        missile: bool,
        linked: Option<&FollowTarget>,
    ) -> Result<(), UnitCompositeError> {
        self.update_at(missile, linked, d2_sim::tick::TickRate::BASE, 0)
    }

    pub fn update_at(
        &mut self,
        missile: bool,
        linked: Option<&FollowTarget>,
        rate: d2_sim::tick::TickRate,
        frame: i32,
    ) -> Result<(), UnitCompositeError> {
        use motion::*;
        if self.flags & DONE != 0 {
            return Ok(());
        }
        let [x, y, z] = &mut self.pos;
        let [vx, vy, vz] = &mut self.vel;
        let [ax, ay, az] = self.acc;
        // Integrate, in this order.
        *x = x.wrapping_add(rate.share(*vx, frame));
        *y = y.wrapping_add(rate.share(*vy, frame));
        *vx = vx.wrapping_add(rate.share(ax, frame));
        *z = z.wrapping_add(rate.share(*vz, frame));
        *vy = vy.wrapping_add(rate.share(ay, frame));
        *vz = vz.wrapping_add(rate.share(az, frame));
        // The timed arc's tick count.
        if self.flags & TIMED != 0 {
            if self.ticks_left == 0 {
                self.flags |= DONE;
                self.pos[0] = 0;
                self.pos[1] = 0;
            } else {
                self.ticks_left -= rate.share(1, frame);
            }
        }
        let [lx, ly, lz] = self.limit;
        if self.flags & BOUNCE != 0 {
            // Bounce at the floor.
            if self.pos[2] >> 11 <= lz {
                self.vel[2] = self
                    .bounce_factor
                    .wrapping_mul(self.vel[2])
                    .wrapping_div(100)
                    .wrapping_neg();
                self.pos[2] = lz;
                if self.bounces_left != 0 {
                    self.bounces_left -= 1;
                } else {
                    self.flags |= DONE;
                }
            }
        } else if self.flags & FOLLOW != 0 {
            // Follow the linked unit; the offset step is skipped.
            let Some(k) = linked else {
                return Ok(());
            };
            if missile {
                if !k.monster {
                    return Err(UnitCompositeError::FollowTarget);
                }
                if CompositeKind::Monster.is_death_mode(k.mode) {
                    return Ok(());
                }
            }
            let (a, b) = k.s7_offset;
            let ox = a.wrapping_add(k.offset[0]);
            let oz = b
                .wrapping_add(if missile { 10 } else { 0 })
                .wrapping_add(k.offset[2]);
            let oy = self.offset[1];
            self.offset[0] = ox;
            self.offset[2] = oz;
            // The follow offset (ox, oy), arithmetic shifts.
            let two_oy = oy.wrapping_mul(2);
            self.pos[0] = two_oy.wrapping_add(ox) >> 5;
            self.pos[1] = two_oy.wrapping_sub(ox) >> 5;
            self.pos[2] = oz.wrapping_neg().wrapping_mul(2048);
            return Ok(());
        } else {
            // Stop at the limit.
            let [a, b, c] = self.pos.map(|p| p >> 11);
            let stop = if self.flags & FROM_BELOW == 0 {
                a <= lx && b <= ly && c <= lz
            } else {
                a >= lx && b >= ly && c >= lz
            };
            if stop {
                self.pos = [lx << 11, ly << 11, lz << 11];
                self.flags |= DONE;
            }
        }
        // The screen offset (flag 0x10 returned above).
        let a = self.pos[0] >> 11;
        let b = self.pos[1] >> 11;
        self.offset = [
            a.wrapping_sub(b) >> 1,
            a.wrapping_add(b) >> 2,
            (self.pos[2] >> 11).wrapping_neg(),
        ];
        Ok(())
    }

    /// Bounce: flag 4, bounces left :=
    /// `count`, factor := `factor` (percent).
    pub fn bounce(&mut self, count: i32, factor: i32) {
        self.flags |= motion::BOUNCE;
        self.bounces_left = count;
        self.bounce_factor = factor;
    }

    /// Clears flag 2 (the timed arc's tick count no
    /// longer ends the record).
    pub fn untimed(&mut self) {
        self.flags &= !motion::TIMED;
    }

    /// Restart: clears flag 1, sets flag 8.
    pub fn restart(&mut self) {
        self.flags = (self.flags & !motion::DONE) | motion::RESTARTED;
    }

    /// Done: sets flag 1.
    pub fn done(&mut self) {
        self.flags |= motion::DONE;
    }

    /// "Still moving" unless flag 1 is set and flag 8
    /// clear.
    pub fn moving(&self) -> bool {
        !(self.flags & motion::DONE != 0 && self.flags & motion::RESTARTED == 0)
    }

    /// Timed arc (height `h`, `n` ticks): flags |= 2 (the other bits, a
    /// restart's flag 8 too, stay), ticks := max(n, 1), x = y = 0,
    /// z := h ≪, vx = vy = 0, az := −0x1000 when 0, vz := (−h·2048 −
    /// trunc(az·n²/2)) / n (C divisions, by the clamped n).
    pub fn timed_arc(&mut self, h: i32, n: i32) {
        let n = n.max(1);
        self.flags |= motion::TIMED;
        self.ticks_left = n;
        self.pos = [0, 0, h.wrapping_shl(11)];
        self.vel[0] = 0;
        self.vel[1] = 0;
        if self.acc[2] == 0 {
            self.acc[2] = -0x1000;
        }
        let az = self.acc[2];
        let fall = az.wrapping_mul(n).wrapping_mul(n) / 2;
        self.vel[2] = h.wrapping_mul(-2048).wrapping_sub(fall) / n;
    }

    /// The draw's addition `(ox, oy + oz)`.
    pub fn draw_offset(&self) -> (i32, i32) {
        let [ox, oy, oz] = self.offset;
        (ox, oy.wrapping_add(oz))
    }
}

/// The table offsets added after the motion record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableOffset {
    None,
    /// `objects` Xoffset, Yoffset, Draw.
    Object {
        x: i32,
        y: i32,
        draw: bool,
    },
    /// `missiles` xoffset, yoffset, zoffset; `None` = no missiles row.
    Missile(Option<(i16, i16, i16)>),
}

/// `ViewSource::unit_offset`: `(ox, oy + oz)` of the record
/// (0 without one) plus the object / missile offsets; `None` when the unit
/// is not drawn (object Draw = 0, missile without a row).
pub fn unit_offset(record: Option<&MotionRecord>, table: TableOffset) -> Option<(i32, i32)> {
    let (x, y) = record.map_or((0, 0), MotionRecord::draw_offset);
    match table {
        TableOffset::None => Some((x, y)),
        TableOffset::Object { draw: false, .. } => None,
        TableOffset::Object {
            x: ox,
            y: oy,
            draw: true,
        } => Some((x.wrapping_add(ox), y.wrapping_add(oy))),
        TableOffset::Missile(None) => None,
        TableOffset::Missile(Some((mx, my, mz))) => Some((
            x.wrapping_add(i32::from(mx)),
            y.wrapping_add(i32::from(my)).wrapping_add(i32::from(mz)),
        )),
    }
}

// ---------------------------------------------------------------------------
// Single-cel units.

/// `DATA\GLOBAL\MISSILES\<CelFile>.dcc` (`CompressedData` = 1).
pub fn missile_file(cel_file: &str) -> String {
    format!("DATA\\GLOBAL\\MISSILES\\{cel_file}.dcc")
}

/// Which graphic an item shows by mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemGraphic {
    Inventory,
    Ground,
}

pub fn item_graphic(mode: u8) -> Option<ItemGraphic> {
    match mode {
        0 | 1 | 2 | 4 | 6 => Some(ItemGraphic::Inventory),
        3 | 5 => Some(ItemGraphic::Ground),
        _ => None,
    }
}

/// A gold pile's direction by the amount of stat 14.
pub fn gold_direction(amount: u32) -> u8 {
    match amount {
        0..=99 => 0,
        100..=499 => 1,
        500..=4999 => 2,
        _ => 3,
    }
}

/// Quality values of single-cel units.
pub const QUALITY_SET: u8 = 5;
pub const QUALITY_UNIQUE: u8 = 7;

/// A ground item's flippy file: the unique / set
/// flippy file when the quality is 7 / 5 and that name is not empty, else
/// the item's own; path `DATA\GLOBAL\items\<name>.dc6`.
pub fn flippy_file(own: &str, quality: u8, unique: Option<&str>, set: Option<&str>) -> String {
    let special = match quality {
        QUALITY_UNIQUE => unique,
        QUALITY_SET => set,
        _ => None,
    }
    .filter(|n| !n.is_empty());
    format!("DATA\\GLOBAL\\items\\{}.dc6", special.unwrap_or(own))
}

#[cfg(test)]
#[path = "unit_composite_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "unit_composite_c2ui_tests.rs"]
mod tests_c2ui;
