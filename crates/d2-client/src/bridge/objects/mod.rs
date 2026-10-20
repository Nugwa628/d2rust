//! The client side of objects: the per-object client update
//! (the generic step, then the object's `ClientFn` and the
//! mode sound call, call site A), the second `ClientFn` call of a C
//! object (call site B), the 18 client object functions ([`fns`]), the
//! interact sender ([`interact`]), set C (client-only
//! units) and the client latches.
//!
//! Plain Rust, no Bevy. Every timer here reads the update's `now`
//! (`ModelInputs::now`); nothing here reaches the server except
//! the C→S messages appended to `outgoing`. The sound and effect calls
//! leave as [`Output::ObjectSound`] / [`Output::ObjectFx`] in update
//! order.

pub mod fns;
pub mod interact;

#[cfg(test)]
mod tests;

pub use crate::app::single_player::rows::ObjClientRow;

use std::collections::BTreeMap;

use d2_sim::rng::Seed;

use super::dispatch::HandlerError;
use super::output::Output;
use super::world::{ClientUnit, ClientWorld, KindData, ModelInputs, MonsterData, UnitKey, OBJECT};

/// Entries of the client object function table.
pub const CLIENT_FNS: u8 = 19;
/// The fatal assert of a `ClientFn` ≥ 19.
pub const FATAL_CLIENT_FN: u32 = 0x546;
/// The fatal assert of an object refresh in a mode without graphics
/// (`Mode<m>` ≠ 1).
pub const FATAL_GFX_MODE: u32 = 0x4DC;
/// The client quest record's size (96 bytes).
pub const QUEST_RECORD: usize = 0x60;

/// With (rec, q, b): bit b of quest q (16 bits per quest, LSB
/// first).
pub fn quest_bit(rec: &[u8; QUEST_RECORD], q: u8, b: u8) -> bool {
    let n = 16 * usize::from(q) + usize::from(b);
    rec.get(n >> 3).is_some_and(|&v| v & (1 << (n & 7)) != 0)
}

/// Unit flag-ex bit of an expansion game.
pub const FLAG_EX_EXPANSION: u32 = 0x0200_0000;

/// What the client object functions read beside the model.
#[derive(Clone, Debug, Default)]
pub struct ObjClientInputs {
    /// One row per `objects.txt` row, by class. Empty: the headless
    /// configuration; the object update then runs nothing (no row to
    /// read).
    pub rows: Vec<ObjClientRow>,
    /// The client quest flag record, held
    /// by the UI layer; `None`: no record
    /// (fatal 0x1F when read).
    pub quest_flags: Option<[u8; QUEST_RECORD]>,
    /// The code of the local player's item in hand (P+0x60)
    /// for the interact sender's classes 404 / 376;
    /// `None`: no item.
    pub hand_code: Option<[u8; 4]>,
    /// By skill id: the `skills` row byte +7 & (= 0x80,
    /// open question 5), i.e. flag bit 31
    /// `interrupt` (`data/fields.tsv` skills 162), read by the action gate.
    /// A skill without an entry: clear.
    pub skill_gate: Vec<bool>,
    /// The byte with (1) of C→S 0x16 (item
    /// case), a UI value.
    pub pickup_flag: u8,
}

/// The client latches, part of the client session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Latches {
    /// The orifice preload ran.
    pub orifice: bool,
    /// The altar preload ran.
    pub altar: bool,
    /// The zoo is on (S→C 0x50 code 36).
    pub zoo: bool,
    /// The zoo has spawned.
    pub zoo_spawned: bool,
    /// The GUID of the last chicken (−1 when its create
    /// failed); only the spawn writes it, the reset leaves it.
    pub last_chicken: u32,
    /// U16@3 of 0x50 code 36 (never read).
    pub zoo_word: u16,
}

impl Latches {
    /// The four bytes are
    /// cleared together; stays.
    pub fn reset(&mut self) {
        self.orifice = false;
        self.altar = false;
        self.zoo = false;
        self.zoo_spawned = false;
    }
}

/// Set C and the object latches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientObjects {
    /// Set C: the client-only units (unit flag 0x200000), by key.
    pub set_c: BTreeMap<UnitKey, ClientUnit>,
    pub latches: Latches,
    /// The client GUID counter: the last
    /// GUID given (.data initial value 1, never
    /// reset; [`create_client_unit`]).
    pub next_guid: u32,
    /// The missile fields of the set-C missiles
    /// (`super::client_missiles`).
    pub missiles: super::client_missiles::ClientMissiles,
    /// The client's state-86 (`justhit`) lists of the client missile end:
    /// the unit and its frames left.
    pub just_hit: BTreeMap<UnitKey, i32>,
    /// The client grids with the unit footprints, as the client missiles
    /// read them (`super::client_missiles::stamp_unit_footprints`), by
    /// room.
    pub unit_grids: BTreeMap<d2_sim::units::RoomId, d2_sim::drlg::CollisionGrid>,
    /// The unit and object footprint bits alone, by room, kept from one
    /// update to the next and stamped / cleared as the units move
    /// (`super::client_missiles::stamp_unit_footprints`).
    pub foot_layer: BTreeMap<d2_sim::units::RoomId, d2_sim::drlg::CollisionGrid>,
    /// The footprint each unit last stamped on [`Self::foot_layer`].
    pub foot_marks: BTreeMap<UnitKey, d2_sim::path::footprint::Footprint>,
    /// Each monster's [`ClientUnit::mode_requests`] at the
    /// last footprint update: a new request re-stamps the footprint.
    ///
    /// [`ClientUnit::mode_requests`]: crate::bridge::world::ClientUnit::mode_requests
    pub foot_requests: BTreeMap<UnitKey, u32>,
    /// The dynamic path records of the C monsters (`super::critter_path`).
    pub c_paths: BTreeMap<UnitKey, d2_sim::path::record::DynamicPath>,
    /// The client path of each server monster (`critter_path`).
    pub s_paths: BTreeMap<UnitKey, d2_sim::path::record::DynamicPath>,
    /// The client missiles' sound calls not yet handed out
    /// (`super::output::Output::MissileSound`).
    pub missile_sounds: Vec<super::client_missiles::MissileSound>,
    /// Monster type flag 0x80 (+0x16) while the umod 29 hook makes its
    /// copies.
    pub multishot_guard: std::collections::BTreeSet<UnitKey>,
    /// `Selectable0`–`7` by object class: the hover pick skips an object
    /// whose current mode is not selectable (the "Dummy" markers under
    /// town NPCs). D2RUST's own, unverified; empty: all picked.
    pub selectable: Vec<[u8; 8]>,
    /// The hover layer test of each object class,
    /// from its `objects.txt` row; empty: no layer test.
    pub hover_layers: Vec<super::hover::ObjectLayerTest>,
    /// `monstats2` `TotalPieces` by monster class;
    /// empty: no layer test.
    pub monster_pieces: Vec<u8>,
}

impl Default for ClientObjects {
    fn default() -> Self {
        Self {
            set_c: BTreeMap::new(),
            latches: Latches::default(),
            next_guid: 1,
            missiles: Default::default(),
            just_hit: BTreeMap::new(),
            unit_grids: BTreeMap::new(),
            foot_layer: BTreeMap::new(),
            foot_marks: BTreeMap::new(),
            foot_requests: BTreeMap::new(),
            c_paths: BTreeMap::new(),
            s_paths: BTreeMap::new(),
            missile_sounds: Vec::new(),
            multishot_guard: Default::default(),
            selectable: Vec::new(),
            hover_layers: Vec::new(),
            monster_pieces: Vec::new(),
        }
    }
}

/// A unit and the set it is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjUnit {
    pub key: UnitKey,
    /// In set C (a client-only unit) rather than set S.
    pub client_only: bool,
}

/// The audio calls of the client object code,
/// in the order the update makes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjSound {
    /// The mode sound call with
    /// U's class and mode at the call.
    Mode {
        unit: ObjUnit,
        class: u32,
        mode: u32,
        /// The path distance from U to P at the call:
        /// the path distance to the local
        /// player, [`NO_LOCAL_DISTANCE`] without one.
        local_dist: i32,
    },
    /// A sound request with (id, U, 0, 0, 0).
    Request { id: i32, unit: ObjUnit },
    /// The player event sound with (P, event) (step 7, classes
    /// 404 / 376 without the item).
    PlayerEvent { player: UnitKey, event: u8 },
}

/// The effect calls of the client object code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjFx {
    /// The graphics refresh with (U, 0) after a mode change.
    GfxRefresh { unit: ObjUnit, mode: u32 },
    /// A graphics load with (class, flag).
    GfxLoad { class: u16, flag: u8 },
    /// Overlay create with (U, overlay, kind, 0, 0, 0, 0, 0).
    OverlayCreate {
        unit: ObjUnit,
        overlay: u16,
        kind: u8,
    },
    /// Overlay remove by id with (U, 0, overlay).
    OverlayRemove { unit: ObjUnit, overlay: u16 },
    /// The object light with (U, Lit / 2, colour)
    /// (the object row; applied by
    /// `rules::lighting::sources::object_light`).
    Light {
        unit: ObjUnit,
        lit: u8,
        rgb: (u8, u8, u8),
    },
    /// The client skill start with (P, R).
    SkillStart { player: UnitKey, record: [i32; 7] },
    /// U flag `+0xC4 |= bits` (`OrderFlag2`: 0x100000, generic step).
    FlagOr { unit: ObjUnit, bits: u32 },
    /// The `Parm7` sound call with (U, id, 4).
    Parm7Sound { unit: ObjUnit, id: u32 },
    /// The collision call on U.
    Collision { unit: ObjUnit },
}

/// The context of one client object call: the model, the inputs, the
/// unit and its class row, and the output list.
pub struct Cx<'a> {
    pub w: &'a mut ClientWorld,
    pub inputs: &'a ModelInputs,
    pub unit: ObjUnit,
    pub row: ObjClientRow,
    pub out: &'a mut Vec<Output>,
}

impl Cx<'_> {
    /// The update's wall clock.
    pub fn now(&self) -> u32 {
        self.inputs.now
    }

    /// U, from its set.
    pub fn u(&mut self) -> Result<&mut ClientUnit, HandlerError> {
        unit_mut(self.w, self.unit).ok_or(HandlerError::Invalid("client object not in its set"))
    }

    /// `step(U)`: one step of U's client seed, the new low word.
    pub fn step(&mut self) -> Result<u32, HandlerError> {
        step_seed(self.u()?)
    }

    /// `range(lo, hi)`.
    pub fn range(&mut self, lo: i32, hi: i32) -> Result<u32, HandlerError> {
        if hi <= lo {
            return Ok(lo as u32);
        }
        let n = hi.wrapping_sub(lo) as u32;
        let r = self.step()?;
        let k = if n.is_power_of_two() {
            r & (n - 1)
        } else {
            r % n
        };
        Ok((lo as u32).wrapping_add(k))
    }

    /// `set_mode(U, m)`: a different mode is
    /// written and the animation re-init [`anim_setup`] runs in it (a new
    /// speed drawn on U's client seed when `Sync` = 0); the same mode
    /// changes nothing the model holds (the 0x0E same-mode
    /// `set_mode` runs no re-init).
    /// TODO: unit flag 1 and the temporary stat
    /// lists are not in the client model.
    pub fn set_mode(&mut self, m: u32) -> Result<(), HandlerError> {
        let row = self.row;
        let u = self.u()?;
        if u.mode != m {
            u.mode = m;
            anim_setup(u, &row, m)?;
        }
        Ok(())
    }

    /// `reinit(U)`: the animation set-up [`anim_setup`] in
    /// U's mode. Measured on the town objects:
    /// draws `roll(d >> 3)` on the object's own seed for
    /// client and server objects alike, whichever caller runs it.
    pub fn reinit(&mut self) -> Result<(), HandlerError> {
        let row = self.row;
        let u = self.u()?;
        let m = u.mode;
        anim_setup(u, &row, m)
    }

    /// `refresh(U)`: an effect call. An object in a
    /// mode whose `Mode<m>` flag (+0x13F + m) is not 1 is fatal 0x4DC
    /// (`Gfx.cpp`; the game ends with exit code 0xFFFFFFFF).
    pub fn refresh(&mut self) -> Result<(), HandlerError> {
        let mode = self.u()?.mode;
        if self.row.mode_ok.get(mode as usize) != Some(&1) {
            return Err(HandlerError::Fatal(FATAL_GFX_MODE));
        }
        self.fx(ObjFx::GfxRefresh {
            unit: self.unit,
            mode,
        });
        Ok(())
    }

    /// `sound(U)`: the mode sound call on U.
    pub fn sound(&mut self) -> Result<(), HandlerError> {
        let u = self.u()?;
        let (class, mode) = (u.class, u.mode);
        let local_dist = local_distance(self.w, self.inputs, self.unit);
        self.out.push(Output::ObjectSound(ObjSound::Mode {
            unit: self.unit,
            class,
            mode,
            local_dist,
        }));
        Ok(())
    }

    /// `End(m)` = `FrameCnt[m]` − 256.
    pub fn end(&self, m: u32) -> Result<i32, HandlerError> {
        let cnt = frame_cnt(&self.row, m)?;
        Ok((cnt as i32).wrapping_sub(256))
    }

    pub fn fx(&mut self, fx: ObjFx) {
        self.out.push(Output::ObjectFx(fx));
    }

    /// The collision call on U: frees U's footprint
    /// on the client grid ([`super::world::ObjectData::footprint`]).
    pub fn free_footprint(&mut self) -> Result<(), HandlerError> {
        let unit = self.unit;
        if let KindData::Object(d) = &mut self.u()?.kind {
            d.footprint = false;
        }
        self.fx(ObjFx::Collision { unit });
        Ok(())
    }
}

/// `FrameCnt[m]`; a mode past 7 reads beyond the eight columns.
fn frame_cnt(row: &ObjClientRow, m: u32) -> Result<u32, HandlerError> {
    row.frame_cnt
        .get(m as usize)
        .copied()
        .ok_or(HandlerError::Invalid(
            "object mode past the eight objects.txt modes",
        ))
}

/// The animation set-up of a client object in mode `m` (the animation
/// init's object branch, on U's client seed): frame := `Start[m]` · 256; speed
/// := `FrameDelta[m]` when `Sync` ≠ 0, else `roll(d >> 3)` + d − (d >> 4)
/// (d = `FrameDelta[m]` read as i16; `roll(n < 1)` draws nothing),
/// clamped to 0..=0x7FFF. A unit without a client seed in the model (no
/// client DRLG) gets no speed (`FrameDelta[m]` then).
pub fn anim_setup(u: &mut ClientUnit, row: &ObjClientRow, m: u32) -> Result<(), HandlerError> {
    let i = m as usize;
    if i >= 8 {
        return Err(HandlerError::Invalid("object mode past 7"));
    }
    u.frame = i32::from(row.start[i]) << 8;
    u.frame_count = row.frame_cnt[i] as i32;
    let d = i32::from(row.frame_delta[i] as i16);
    if row.sync != 0 {
        u.speed = Some(d);
        return Ok(());
    }
    let Some((lo, hi)) = u.seed else {
        u.speed = None;
        return Ok(());
    };
    let mut s = Seed::new(lo, hi);
    let r = s.roll(d >> 3) as i32;
    u.seed = Some((s.lo, s.hi));
    u.speed = Some(r.wrapping_add(d).wrapping_sub(d >> 4).clamp(0, 0x7FFF));
    Ok(())
}

/// One step of a unit's client seed; the new low word.
pub fn step_seed(u: &mut ClientUnit) -> Result<u32, HandlerError> {
    let (lo, hi) = u.seed.ok_or(HandlerError::Invalid(
        "client seed not in the model (no client DRLG)",
    ))?;
    let mut s = Seed::new(lo, hi);
    let r = s.step();
    u.seed = Some((s.lo, s.hi));
    Ok(r)
}

/// The unit of `r`, from set S or set C.
pub fn unit_mut(w: &mut ClientWorld, r: ObjUnit) -> Option<&mut ClientUnit> {
    if r.client_only {
        w.objclient.set_c.get_mut(&r.key)
    } else {
        w.units.get_mut(&r.key)
    }
}

pub fn unit_ref(w: &ClientWorld, r: ObjUnit) -> Option<&ClientUnit> {
    if r.client_only {
        w.objclient.set_c.get(&r.key)
    } else {
        w.units.get(&r.key)
    }
}

/// U's class row; `None` in the headless configuration (no rows).
fn row_of(
    w: &ClientWorld,
    inputs: &ModelInputs,
    r: ObjUnit,
) -> Result<Option<ObjClientRow>, HandlerError> {
    let rows = &inputs.objclient.rows;
    if rows.is_empty() {
        return Ok(None);
    }
    let Some(u) = unit_ref(w, r) else {
        return Ok(None);
    };
    rows.get(u.class as usize)
        .copied()
        .map(Some)
        .ok_or(HandlerError::Invalid(
            "object class without an objects.txt row",
        ))
}

/// The classes whose `Overlay` the end of mode 1 creates.
const END_OVERLAY_CLASSES: [u32; 7] = [354, 355, 356, 397, 405, 406, 407];

/// The generic object step. The speed is U's own (+0x4C,
/// [`anim_setup`]); a unit without one (no setup ran) steps by the
/// class's `FrameDelta[mode]`.
///
/// A cycling mode wraps by one subtraction of `FrameCnt` (Start 0):
/// as for the Rogue Encampment torches and classes 35, 36, 39,
/// 40–42.
pub fn generic_step(cx: &mut Cx<'_>) -> Result<(), HandlerError> {
    let m = cx.u()?.mode;
    let cnt = frame_cnt(&cx.row, m)?;
    if cnt == 0x100 {
        return Ok(());
    }
    let i = m as usize;
    let delta = i32::from(cx.row.frame_delta[i]);
    let class = cx.u()?.class;
    if cx.row.cycle_anim[i] == 0 {
        if cx.row.is_door != 0 {
            return door_step(cx, cnt as i32);
        }
        let f = cx.u()?.frame;
        if f >= (cnt as i32).wrapping_sub(256) {
            if class == 189 && matches!(m, 2 | 3) {
                let n = m + 1;
                cx.u()?.mode = n;
                cx.refresh()?;
                cx.reinit()?;
                cx.u()?.frame = i32::from(cx.row.start[n as usize]) * 256;
                return Ok(());
            }
            if m != 1 {
                return Ok(());
            }
            return end_of_mode_1(cx, class);
        }
    }
    let start = i32::from(cx.row.start[i]) * 256;
    let cycle = cx.row.cycle_anim[i] != 0;
    let (rate, tick) = (cx.w.tick_rate, cx.w.share_frame());
    let u = cx.u()?;
    // The speed is 8.8 frames per base-rate tick: this tick's share of it.
    let speed = rate.share(u.speed.unwrap_or(delta), tick);
    let c = cnt as i32;
    if class == 12 {
        u.frame = u.frame.wrapping_sub(speed);
        if u.frame < 0 {
            u.frame = u.frame.wrapping_add(c);
        }
    } else if class == 189 && m == 3 {
        u.frame = u.frame.wrapping_sub(speed);
        if u.frame < 0 {
            u.mode = 4;
            cx.refresh()?;
            cx.reinit()?;
            cx.u()?.frame = i32::from(cx.row.start[4]) * 256;
        }
    } else {
        u.frame = u.frame.wrapping_add(speed);
        if u.frame >= c {
            u.frame = if cycle {
                start.wrapping_add(u.frame.wrapping_sub(c))
            } else {
                c.wrapping_sub(256)
            };
        }
    }
    Ok(())
}

/// The door step (settled): `End` =
/// `FrameCnt[m] − 256`; mode 1 opens (at `End` → finish, else the frame
/// advances by the speed, clamped to `End`), mode 3 closes (at or below 0
/// → finish, else it runs back, clamped to 0); any other mode is fatal.
fn door_step(cx: &mut Cx<'_>, cnt: i32) -> Result<(), HandlerError> {
    let m = cx.u()?.mode;
    let end = cnt.wrapping_sub(256);
    let (rate, tick) = (cx.w.tick_rate, cx.w.share_frame());
    let u = cx.u()?;
    let s = u
        .speed
        .unwrap_or_else(|| i32::from(cx.row.frame_delta[m as usize & 7]));
    // Per base-rate tick: this tick's share of it.
    let s = rate.share(s, tick);
    let u = cx.u()?;
    match m {
        1 => {
            if u.frame != end {
                u.frame = u.frame.wrapping_add(s).min(end);
                return Ok(());
            }
        }
        3 => {
            if u.frame > 0 {
                u.frame = if s > u.frame { 0 } else { u.frame - s };
                return Ok(());
            }
        }
        _ => {
            return Err(HandlerError::Invalid(
                "door step in a mode other than 1 or 3",
            ))
        }
    }
    // Finish: no sound, light, OrderFlag2, Parm7 or overlay.
    if m == 3 {
        cx.u()?.mode = 0;
        cx.u()?.frame = i32::from(cx.row.start[0]);
    } else {
        cx.free_footprint()?;
        cx.u()?.mode = 2;
        cx.u()?.frame = i32::from(cx.row.start[2]);
    }
    cx.refresh()?;
    cx.reinit()?;
    let n = cx.u()?.mode as usize;
    let sel = cx.row.selectable[n] != 0;
    cx.u()?.flag_2 = Some(sel);
    Ok(())
}

/// Mode 1: the update after the clamp turns it into mode 2.
fn end_of_mode_1(cx: &mut Cx<'_>, class: u32) -> Result<(), HandlerError> {
    let unit = cx.unit;
    cx.u()?.mode = 2;
    if cx.row.order_flag2 == 1 {
        cx.u()?.flag_flat = true;
        cx.fx(ObjFx::FlagOr {
            unit,
            bits: 0x10_0000,
        });
    }
    if cx.row.parm7 != 0 {
        let id = if cx.row.parm7 == 0xFF { 0x153 } else { 0x97 };
        cx.fx(ObjFx::Parm7Sound { unit, id });
    }
    cx.u()?.frame = i32::from(cx.row.start[2]) * 256;
    cx.refresh()?;
    cx.reinit()?;
    cx.u()?.frame = i32::from(cx.row.start[2]) * 256;
    if cx.row.overlay != 0 && END_OVERLAY_CLASSES.contains(&class) {
        cx.fx(ObjFx::OverlayRemove {
            unit,
            overlay: 0x47,
        });
    }
    let sel = cx.row.selectable[2] != 0;
    cx.u()?.flag_2 = Some(sel);
    let (lit, rgb) = (cx.row.lit[2], cx.row.rgb);
    cx.fx(ObjFx::Light { unit, lit, rgb });
    if cx.row.has_collision[2] == 0 && cx.row.has_collision[1] != 0 {
        cx.free_footprint()?;
    }
    Ok(())
}

/// The dispatch: `ClientFn` 0 → 1 without a call;
/// ≥ 19 → fatal 0x546; else the table entry's result.
pub fn dispatch(cx: &mut Cx<'_>) -> Result<bool, HandlerError> {
    fns::call(cx)
}

/// The object update (call site A;
/// step 2): the generic step, then `ClientFn` ≤ 3 → the mode sound call
/// only; ≥ 4 → the dispatch, then the mode sound call when it returned
/// non-zero. Nothing without object rows.
pub fn object_update(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    unit: ObjUnit,
    out: &mut Vec<Output>,
) -> Result<(), HandlerError> {
    let Some(row) = row_of(w, inputs, unit)? else {
        return Ok(());
    };
    let mut cx = Cx {
        w,
        inputs,
        unit,
        row,
        out,
    };
    generic_step(&mut cx)?;
    if row.client_fn <= 3 {
        return cx.sound();
    }
    if dispatch(&mut cx)? {
        cx.sound()?;
    }
    Ok(())
}

/// Call site B: after a C unit's update,
/// a type-2 unit still in set C runs the dispatch once more, result
/// ignored. (Type 1 runs the critter AI in the C monsters'
/// walk, [`super::critters::c_monsters`].)
pub fn site_b(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
    out: &mut Vec<Output>,
) -> Result<(), HandlerError> {
    if key.unit_type != OBJECT || !w.objclient.set_c.contains_key(&key) {
        return Ok(());
    }
    let unit = ObjUnit {
        key,
        client_only: true,
    };
    let Some(row) = row_of(w, inputs, unit)? else {
        return Ok(());
    };
    let mut cx = Cx {
        w,
        inputs,
        unit,
        row,
        out,
    };
    dispatch(&mut cx)?;
    Ok(())
}

/// The C set walk as a cursor:
/// buckets `GUID & 0x7F` ascending; a bucket's head is read when the walk
/// reaches it and each unit's chain link before the unit runs. So a unit
/// created during the walk is visited in the same walk when its bucket
/// comes later (a new GUID is the highest, so it goes to its chain's head);
/// one in the current or an earlier bucket is not.
pub struct CWalk {
    unit_type: u8,
    bucket: u32,
    next: Option<u32>,
}

impl CWalk {
    pub fn new(unit_type: u8) -> Self {
        Self {
            unit_type,
            bucket: 0,
            next: None,
        }
    }

    /// The next unit to run; its chain link is read now, before it runs.
    pub fn next(&mut self, w: &ClientWorld) -> Option<UnitKey> {
        let t = self.unit_type;
        let in_chain = |b: u32, below: Option<u32>| {
            w.objclient
                .set_c
                .keys()
                .filter(|k| {
                    k.unit_type == t && k.guid & 0x7F == b && below.is_none_or(|g| k.guid < g)
                })
                .map(|k| k.guid)
                .max()
        };
        let guid = match self.next.take() {
            Some(g) => g,
            None => loop {
                if self.bucket >= 0x80 {
                    return None;
                }
                let b = self.bucket;
                self.bucket += 1;
                if let Some(g) = in_chain(b, None) {
                    break g;
                }
            },
        };
        self.next = in_chain(guid & 0x7F, Some(guid));
        Some(UnitKey::new(t, guid))
    }
}

/// The order of a C set walk:
/// buckets `GUID & 0x7F` ascending, each chain in descending GUID.
pub fn c_order(w: &ClientWorld, unit_type: u8) -> Vec<UnitKey> {
    let mut keys: Vec<UnitKey> = w
        .objclient
        .set_c
        .keys()
        .copied()
        .filter(|k| k.unit_type == unit_type)
        .collect();
    keys.sort_by_key(|k| (k.guid & 0x7F, std::cmp::Reverse(k.guid)));
    keys
}

/// The client-only creator with (class, x, y, …) (flags 0x600000):
/// a new unit in set C with a GUID from the client
/// counter and the creation fields of the client model (step 6).
/// Returns its key; `None` when the create fails (the point is in no
/// room of the client DRLG).
///
/// The GUID is the counter + 1 (−1 wraps to 0), stored back before the
/// create: the first client GUID is 2, and a failed
/// create still uses its GUID.
pub fn create_client_unit(
    w: &mut ClientWorld,
    unit_type: u8,
    class: u32,
    x: u16,
    y: u16,
) -> Option<UnitKey> {
    let guid = w.objclient.next_guid.wrapping_add(1);
    w.objclient.next_guid = guid;
    let key = UnitKey::new(unit_type, guid);
    let mut u = ClientUnit::new(key);
    u.class = class;
    if w.expansion != 0 {
        u.flag_ex |= FLAG_EX_EXPANSION;
    }
    let placed = (x, y) != (0, 0);
    if placed && w.drlg.is_some() {
        let r = w.room_at(x, y)?;
        let seed = w.drlg.as_mut().and_then(|d| d.unit_seed(r.room))?;
        u.seed = Some((seed.lo, seed.hi));
    } else if placed {
        // No client DRLG: the room's seed is not in the model
        // (`msg/units.rs` `create`).
        u.seed = None;
    }
    u.position = placed.then_some((x, y));
    if unit_type == super::world::MONSTER {
        u.kind = KindData::Monster(Box::new(MonsterData {
            hc_idx: class as u16,
            ..MonsterData::default()
        }));
    }
    // Insert fails on a key already in the set.
    if w.objclient.set_c.contains_key(&key) {
        return None;
    }
    w.objclient.set_c.insert(key, u);
    Some(key)
}

/// The client-only removal with (GUID, type) (step 5): unlink from set
/// C and free; a key not in set C: nothing. The free is recorded for
/// its `UnitFreed` output (every unit free, set C too).
pub fn remove_client_unit(w: &mut ClientWorld, key: UnitKey) -> Option<ClientUnit> {
    let u = w.objclient.set_c.remove(&key)?;
    w.objclient.c_paths.remove(&key);
    w.objclient.missiles.remove(&key);
    w.room_units.leave(super::world::view_key(key));
    w.freed.push((key, true));
    Some(u)
}

/// The path distance from a to b over client units:
/// per axis |Δ| − (size(a) / 2 + size(b) / 2), floored at 0, then
/// (2·max + min) / 2. Positions are the client cells.
pub fn distance(a: &ClientUnit, size_a: i32, b: &ClientUnit, size_b: i32) -> i32 {
    distance_at(a.cell(), size_a, b.cell(), size_b)
}

/// [`distance`] between two cells.
pub fn distance_at(a: (u16, u16), size_a: i32, b: (u16, u16), size_b: i32) -> i32 {
    let ((ax, ay), (bx, by)) = (a, b);
    d2_sim::world::objects::chests::reach_distance(
        (i32::from(ax), i32::from(ay)),
        size_a,
        (i32::from(bx), i32::from(by)),
        size_b,
    )
}

/// The unit distance between cells
/// with sizes; no path tables: far.
pub fn unit_distance_at(a: (u16, u16), size_a: i32, b: (u16, u16), size_b: i32) -> i32 {
    let Some(t) = super::predict::path_tables() else {
        return i32::MAX;
    };
    let p = |(x, y): (u16, u16)| d2_sim::path::coords::Point::new(i32::from(x), i32::from(y));
    d2_sim::path::walk::geom::unit_distance(t, p(a), size_a, p(b), size_b)
}

/// The distance of an [`ObjSound::Mode`] without a local player (or with
/// U gone): no rule reads it as near. The base game has no value for
/// this case: P is passed with no null test to the distance, which
/// reads P's type unguarded, so a pass without a local player faults
/// there. D2RUST reads it as far: a guard for a state the base game
/// never reaches, with no request.
pub const NO_LOCAL_DISTANCE: i32 = i32::MAX;

/// The path distance for an object U and the local player P (the mode
/// sound's distance).
pub fn local_distance(w: &ClientWorld, inputs: &ModelInputs, unit: ObjUnit) -> i32 {
    let rows = &inputs.objclient.rows;
    let u = if unit.client_only {
        w.objclient.set_c.get(&unit.key)
    } else {
        w.units.get(&unit.key)
    };
    match (u, w.local()) {
        (Some(u), Some(p)) => distance(u, unit_size(u, rows), p, unit_size(p, rows)),
        _ => NO_LOCAL_DISTANCE,
    }
}

/// The unit size for a client unit:
/// player 2, object `SizeX`, item 1, tile 0.
/// TODO: monster and missile sizes need
/// `monstats2` / `missiles` columns the client tables do not hold; 0.
pub fn unit_size(u: &ClientUnit, rows: &[ObjClientRow]) -> i32 {
    match u.key.unit_type {
        super::world::PLAYER => 2,
        OBJECT => rows.get(u.class as usize).map_or(0, |r| r.size_x as i32),
        super::world::ITEM => 1,
        _ => 0,
    }
}
