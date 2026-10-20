//! Client missiles and cast overlays in the play preview's world view: the
//! skill messages S→C 0x4C / 0x4D (mode request codes 0x16 / 0x15, kept
//! as the unit's `last_cast`) start the skill's cast overlay; the model's
//! client missiles draw at their path positions; the units' state
//! overlays play on their units. Everything is drawn only: the
//! server's missile decides every outcome (step 7).
//!
//! What follows the specs:
//! - the real client creates its own missiles from the skill messages
//!   (the server sends none, 0x73 aside);
//! - the art: `missiles` CelFile, one cel of one file by direction and
//!   frame; offsets `xoffset`, `yoffset` +
//!   `zoffset`; camera position of a moving unit;
//! - the draw mode: `missiles.Trans` / `overlay.Trans` through the cel ops;
//! - the draw order: a missile in the unit list of its cell, an overlay
//!   with its host unit ([`Missiles::keyed`]);
//!   a missile of `CltDoFunc` 2 or 11 past its
//!   animation end is flat (flag 0x10000) and
//!   files in its cell's shadow list ([`flat_at_end`]); a missile the
//!   sight test hides is not drawn.
//!
//! D2RUST's own, unverified (the preview's fills):
//! - a flat missile appends after its cell's shadow entries (it is in no
//!   room unit list); the effect's lifetime stays the layer's own (the
//!   functions 2 and 11 keep the missile at its end; not modelled);
//! - the cast overlay: the skill's `castoverlay` at the unit's cast
//!   request (players and monsters); the skill's `cltmissile` itself is
//!   the model's client missile, created by the client do on the action
//!   frame (a monster's from its skill step)
//!   and drawn here from `objclient`;
//! - the art: `data\global\missiles\<CelFile>.dcc`, else `.dc6`; overlays
//!   `data\global\overlays\<Filename>.dcc`, else `.dc6` (file names only
//!   from the `Missiles.txt` / `Overlay.txt` columns); frame
//!   `AnimRate` (per base-rate tick) over the age at the game's rate, `>> 8`
//!   (looping when `LoopAnim`, else the last frame);
//! - no light (light byte 0xFF); a missile is never flat (unit flag
//!   0x10000 not modeled) and not sight-tested;
//! - without a draw order (no map feed): draw key pass 6 after every
//!   other unit;
//! - state overlays: each of the unit's states with an `overlay1` plays
//!   looping on the unit; `ModelFeed` states the unit's position.
//!
//! A file that is missing or fails to parse is logged once and not drawn.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use d2_data::tables::{Missiles as MissileTable, Monstats, Monstats2, Overlay, Skills, States};
use d2_sim::tick::TickRate;

use crate::assets::path::{CanonicalPath, FileSource};
use crate::audio::driver::ClientSeed;
use crate::bridge::client_missiles::FLAG_EX_NOT_DRAWN;
use crate::bridge::hit_overlays::HitOverlay;
use crate::bridge::output::{Output, ShrineFxKind};
use crate::bridge::predict::cell_centre;
use crate::bridge::world::{
    view_key, ClientUnit, ClientWorld, ModeRequest, UnitKey, MISSILE, MONSTER, PLAYER,
    VIEW_CLIENT_BIT,
};
use crate::frames::{FramePart, FrameSet, FrameSetKey};
use crate::rules::blend::{cel_ops, missile_mode, overlay_mode, MODE_OPAQUE};
use crate::rules::camera::{moving_to_client, Camera, ClientPos, FrameSize, UnitPosition};
use crate::rules::draw_order::{tile_of, DrawGrid, UnitSlot};
use crate::rules::placement::place;
use crate::rules::unit_composite::{file_direction, unit_offset, TableOffset};
use crate::scene::order::pass;
use crate::scene::{BlendOp, DrawItem, DrawKey, ItemTag, Rect, ShadeChain};
use crate::ui::original::msg_ui::OverlayCall;

use super::overlay::{
    kind6_clock_ms, overlay_draws, HeightUnit, OverlayEnv, OverlayList, OverlayRow as OverlayRules,
};

use super::feed::ViewFeed;
use super::{ViewAssets, WorldFrame};

/// The mode request codes of the client skill start (`modes.rs`): to a
/// point (S→C 0x4D) and to a unit (S→C 0x4C).
const CAST_POINT: u8 = 0x15;
const CAST_UNIT: u8 = 0x16;
/// Ticks one frame step may cover before the layer restarts its clock.
const MAX_CATCH_UP: u64 = 64;
/// The light byte of an unlit cel.
const UNLIT: u8 = 0xFF;
/// The tag base of an effect's draw (the unit tag space is the GUIDs).
const TAG_BASE: u32 = 0xE000_0000;

/// One `missiles` row as the client reads it. D2RUST's own, unverified.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MissileRow {
    pub cel_file: String,
    pub vel: u32,
    pub range: u32,
    pub anim_rate: u32,
    pub anim_len: u32,
    pub loop_anim: bool,
    /// `xoffset`, `yoffset`, `zoffset`.
    pub offset: (i16, i16, i16),
    /// `explosionmissile` when `Explosion`, else 0 (none).
    pub explosion: u16,
    /// `Trans` (`+0x18D`): the draw mode.
    pub trans: u8,
    /// `pCltDoFunc` (+0x08): the client update function;
    /// 2 and 11 make the missile flat at its
    /// animation end.
    pub clt_do_func: u16,
    /// `InitSteps`: the missile is not drawn until its elapsed updates
    /// exceed it.
    pub init_steps: u32,
}

/// One `overlay` row as the client reads it. D2RUST's own, unverified.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OverlayRow {
    pub file: String,
    pub frames: u32,
    pub anim_rate: u32,
    pub offset: (i32, i32),
    /// `Trans` (`+0x7C`): the draw mode.
    pub trans: u8,
    /// `PreDraw` (`+0x48`): drawn in the back call, before the host's
    /// slot 0.
    pub pre_draw: bool,
}

/// The rows the effect layer reads, by id (0 = none).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectRows {
    /// `cltmissile` by skill id.
    pub skill_missile: Vec<u16>,
    /// `castoverlay` by skill id.
    pub skill_overlay: Vec<u16>,
    pub missiles: Vec<MissileRow>,
    pub overlays: Vec<OverlayRow>,
    /// The states with the `aura` flag (the kind-8 cycle
    /// of the overlay advance).
    pub aura_states: BTreeSet<u8>,
    /// The `overlay.txt` columns of the create and update rules,
    /// by id.
    pub overlay_rules: Vec<OverlayRules>,
    /// By `monstats` class: the `monstats2` row's `noOvly` and
    /// `overlayHeight` (`None`: no `monstats2` row).
    pub monster_overlay: Vec<(bool, Option<u8>)>,
    /// = 0 (no `d2exp.mpq`).
    pub classic_install: bool,
}

fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
}

impl EffectRows {
    /// The rows of the decoded tables.
    pub fn from_tables(
        skills: &[Skills],
        missiles: &[MissileTable],
        overlays: &[Overlay],
        states: &[States],
    ) -> Self {
        EffectRows {
            skill_missile: skills.iter().map(|s| s.cltmissile).collect(),
            skill_overlay: skills.iter().map(|s| s.castoverlay).collect(),
            missiles: missiles
                .iter()
                .map(|m| MissileRow {
                    cel_file: text(&m.celfile),
                    vel: u32::from(m.vel),
                    range: u32::from(m.range),
                    // `AnimSpeed` << 4 wins over `animrate` (clamped to
                    // i16).
                    anim_rate: if m.animspeed != 0 {
                        (u32::from(m.animspeed) << 4).min(0x7FFF)
                    } else {
                        u32::from(m.animrate)
                    },
                    anim_len: u32::from(m.animlen),
                    loop_anim: m.loopanim != 0,
                    offset: (m.xoffset as i16, m.yoffset as i16, m.zoffset as i16),
                    // `ExplosionMissile` (i16, blank = -1) ≥ 0 is the effect
                    // the end function creates;
                    // the `Explosion` column marks a missile that is itself
                    // an explosion, not the link.
                    explosion: if (m.explosionmissile as i16) >= 0 {
                        m.explosionmissile
                    } else {
                        0
                    },
                    trans: m.trans,
                    clt_do_func: m.pcltdofunc,
                    init_steps: u32::from(m.initsteps),
                })
                .collect(),
            overlays: overlays
                .iter()
                .map(|o| OverlayRow {
                    file: text(&o.filename),
                    frames: o.frames,
                    anim_rate: o.animrate,
                    offset: (o.xoffset as i32, o.yoffset as i32),
                    trans: o.trans,
                    pre_draw: o.predraw != 0,
                })
                .collect(),
            aura_states: states
                .iter()
                .filter(|s| s.aura)
                .filter_map(|s| u8::try_from(s.state).ok())
                .collect(),
            overlay_rules: overlays
                .iter()
                .map(|o| OverlayRules {
                    version: o.version as i16,
                    pre_draw: o.predraw != 0,
                    xoffset: o.xoffset as i32,
                    yoffset: o.yoffset as i32,
                    height: [o.height1, o.height2, o.height3, o.height4].map(|h| h as i32),
                    anim_rate: o.animrate as i32,
                    init_radius: o.initradius as i32,
                    radius: o.radius as i32,
                    loop_wait_time: o.loopwaittime as i32,
                })
                .collect(),
            monster_overlay: Vec::new(),
            classic_install: false,
        }
    }

    /// The rows with the monsters' `monstats2` overlay columns: a class's
    /// `monstats2` row is its `monstats` `MonStatsEx` (+0x18) index.
    pub fn with_monsters(mut self, monstats: &[Monstats], monstats2: &[Monstats2]) -> Self {
        self.monster_overlay = monstats
            .iter()
            .map(|m| {
                monstats2
                    .get(usize::from(m.monstatsex))
                    .map_or((false, None), |r| (r.noovly, Some(r.overlayheight)))
            })
            .collect();
        self
    }

    fn is_empty(&self) -> bool {
        self.missiles.is_empty() && self.overlays.is_empty()
    }
}

/// Where an effect's art lives and how it plays.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Art {
    /// Archive paths to try, in order.
    files: [String; 2],
    /// 8.8 frames per tick (256 when the row states none).
    rate: u32,
    /// Ticks to live when it is not a flight (`life`).
    loops: bool,
    offset: (i32, i32),
    /// The draw mode.
    mode: u8,
    /// Overlays: `PreDraw` (back call); `None` for missiles.
    pre_draw: Option<bool>,
}

/// One live effect.
#[derive(Clone, Debug)]
struct Fx {
    art: Art,
    /// 16.16 subtiles.
    at: (u32, u32),
    /// Follows this unit's cell instead of `at`.
    follow: Option<UnitKey>,
    dir64: u8,
    born: u64,
    /// Ticks to live.
    life: u64,
    id: u32,
    /// The `missiles` row of a missile effect; `None` for a cast
    /// overlay (the only effect the layer starts itself now: client
    /// missiles are the model's, drawn from `objclient`).
    missile: Option<u16>,
    /// `CltDoFunc` and `AnimLen` of the missile row (0 for overlays).
    clt_do_func: u16,
    anim_len: u32,
    /// `InitSteps` of the missile row: not drawn until the age exceeds it.
    init_steps: u64,
    /// The creation order shared with the unit overlay records
    /// ([`super::overlay::next_seq`]).
    seq: u64,
}

impl Fx {
    /// Unit flag 0x10000 of the client missile ([`flat_at_end`]) at this
    /// effect's age at the game's rate `at`.
    fn flat(&self, age: u64, at: TickRate) -> bool {
        flat_at_end(self.clt_do_func, self.anim_len, self.art.rate, age, at)
    }
}

/// Whether a client missile has unit flag 0x10000 at `age` ticks of the
/// game's rate `at`: functions 2 (blood) and 11 set it at the
/// animation end (frame + speed ≥ length, frame = the 8.8 speed per
/// base-rate tick over the age, length `AnimLen` << 8); a flat missile
/// files as a flat unit.
pub fn flat_at_end(clt_do_func: u16, anim_len: u32, rate: u32, age: u64, at: TickRate) -> bool {
    matches!(clt_do_func, 2 | 11)
        && at.progress(u64::from(rate), age.saturating_add(1)) >= u64::from(anim_len) << 8
}

/// A file made resident: its archive path, directions and frames.
#[derive(Clone, Debug)]
struct Resident {
    path: CanonicalPath,
    directions: u8,
    frames: usize,
}

/// What [`Missiles::draw_one`] places.
struct Placing<'a> {
    art: &'a Art,
    at: (u32, u32),
    dir64: u8,
    /// 8.8 frames played since the effect was born (its `AnimRate` over
    /// its age at the game's rate).
    progress: u64,
    /// `u64::MAX`: loops for as long as it is drawn.
    life: u64,
    tag: u32,
    /// A record's own frame (`f = +0x18 >> 8`) instead of the age's;
    /// not drawn when the file has no such frame.
    frame: Option<usize>,
    /// The light byte `v` of the cel draw.
    light: u8,
    /// The client motion record's `(ox, oy + oz)`,
    /// added before the art's offsets; (0, 0) without a record.
    motion: (i32, i32),
}

/// One drawn effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissileDraw {
    pub id: u32,
    pub item: DrawItem,
    join: Join,
}

/// Where an effect joins the frame's draw order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Join {
    /// An overlay drawn with its host unit (:
    /// back overlays sub 0 before slot 0, front ones sub 255).
    Host { host: UnitKey, back: bool },
    /// A missile: a room unit filed in the unit list of its cell,
    /// at client position `at`.
    Cell { at: ClientPos },
    /// A flat missile (unit flag 0x10000): filed in its cell's shadow
    /// list, drawn in the shadow pass after the cell's entries.
    Flat { at: ClientPos },
}

/// The effect layer of the world view: the rows (handed in by the app),
/// the file source, the live effects and the files loaded so far. The
/// default draws nothing.
#[derive(Default)]
pub struct Missiles {
    rows: EffectRows,
    source: Option<Arc<dyn FileSource>>,
    files: BTreeMap<String, Option<Resident>>,
    live: Vec<Fx>,
    /// The cast request last seen per unit.
    seen: BTreeMap<UnitKey, (ModeRequest, u32)>,
    started: bool,
    now: u64,
    next_id: u32,
    last: Vec<MissileDraw>,
    /// The overlay records of the UI's calls by host (the unit's
    /// graphics-record list).
    unit_overlays: BTreeMap<UnitKey, OverlayList>,
    /// The overlay calls not run yet, in tick order, each with the
    /// server tick of its delivery, run before the update advance of
    /// their tick: the UI's (kind 3), the bridge's effect outputs
    /// ([`Self::effect_output`]) and the model's state overlays (kinds 3,
    /// 4, 8, 9 and removals).
    overlay_calls: Vec<HitOverlay>,
    /// The model's hit overlay creates not run yet (kind 2),
    /// in tick order, run after the update
    /// advance of their tick: the request queue drains, and the
    /// per-type update sets modes, after the unit's overlay update,
    /// so frame 0 draws in that tick.
    late_calls: Vec<HitOverlay>,
    /// The hit overlay count last seen per unit
    /// (`ClientUnit::hit_overlays`).
    seen_hits: BTreeMap<UnitKey, u32>,
    /// The frame count of each created overlay's file, loaded
    /// before the update advance runs the calls.
    overlay_frames: BTreeMap<u16, Option<i32>>,
    /// The local player's client seed shared with the sound and cursor
    /// draws ([`Missiles::with_client_seed`]); unlinked, the ground
    /// overlay keeps the start the bridge drew on the model's copy.
    client_seed: Option<Arc<Mutex<ClientSeed>>>,
}

/// The item's ground overlay id (overlay.txt row 70, `gleam`), made by the
/// item update.
const GROUND_OVERLAY: u16 = 70;

impl std::fmt::Debug for Missiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Missiles")
            .field("live", &self.live.len())
            .field("files", &self.files.len())
            .field("last", &self.last)
            .finish()
    }
}

impl Missiles {
    /// The layer with the effect rows and the archives to read art from.
    pub fn new(source: Arc<dyn FileSource>, rows: EffectRows) -> Self {
        Missiles {
            rows,
            source: Some(source),
            ..Missiles::default()
        }
    }

    /// Links the local player's client seed (`audio::driver::SoundLink`):
    /// the ground overlay's start is drawn on it.
    pub fn with_client_seed(mut self, seed: Arc<Mutex<ClientSeed>>) -> Self {
        self.client_seed = Some(seed);
        self
    }

    /// Queues the UI's overlay calls of one delivery at server tick
    /// `tick`, in call order; the update advance runs them just before
    /// that tick's step, however many ticks the next
    /// [`Self::add_to_frame`] catches up (a
    /// record created before the walk runs in the same update).
    pub fn overlay_calls(&mut self, tick: u64, calls: impl IntoIterator<Item = OverlayCall>) {
        for c in calls {
            self.queue_call(HitOverlay {
                tick,
                unit: c.unit,
                id: c.id,
                kind: 3,
                args: [0; 3],
                remove: !c.on,
            });
        }
    }

    /// Queues the model's state overlay calls, each run just before its
    /// tick's step as the UI's
    /// calls are. The hit overlays come from
    /// `ClientUnit::last_hit_overlay` and run after the step.
    pub fn hit_overlays(&mut self, calls: impl IntoIterator<Item = HitOverlay>) {
        for c in calls {
            self.queue_call(c);
        }
    }

    /// The effect layer's share of one bridge effect output delivered at
    /// server tick `tick`, in list order:
    /// S→C 0x11 `UnitOverlay` queues with (U, overlay, mode, 0, …),
    /// and the shrine on-mode function of
    /// S→C 0x0E code 3 and 0x51 queues its
    /// removes and creates, run like the UI's calls before the next update
    /// advance. Returns false for an effect output this layer
    /// does not consume yet.
    pub fn effect_output(&mut self, tick: u64, o: &Output) -> bool {
        match *o {
            Output::UnitOverlay {
                unit,
                overlay,
                mode,
                ..
            } => {
                self.queue_call(HitOverlay {
                    tick,
                    unit,
                    id: overlay,
                    kind: mode,
                    args: [0; 3],
                    remove: false,
                });
                true
            }
            // The shrine on-mode function:
            // for each of the entry's two overlay ids that is
            // not −1, a remove by id, then a create
            // (kind 3) when the object is in mode 0.
            Output::ShrineFx {
                kind: ShrineFxKind::OnMode,
                object,
                overlays,
                mode,
                ..
            } => {
                for id in overlays.into_iter().filter_map(|id| u16::try_from(id).ok()) {
                    let call = |remove| HitOverlay {
                        tick,
                        unit: object,
                        id,
                        kind: 3,
                        args: [0; 3],
                        remove,
                    };
                    self.queue_call(call(true));
                    if mode == 0 {
                        self.queue_call(call(false));
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Queues one create or remove after the calls queued for its tick
    /// or earlier (the queue stays in tick order).
    fn queue_call(&mut self, call: HitOverlay) {
        let at = self.overlay_calls.partition_point(|c| c.tick <= call.tick);
        self.overlay_calls.insert(at, call);
    }

    /// The overlay records on `unit`, list head first.
    pub fn unit_overlays(&self, unit: UnitKey) -> &[super::overlay::Overlay] {
        self.unit_overlays
            .get(&unit)
            .map_or(&[], |l| l.records.as_slice())
    }

    /// The last drawn frame's effects, in draw order.
    pub fn last(&self) -> &[MissileDraw] {
        &self.last
    }

    /// The number of live effects (flights, explosions, cast overlays).
    pub fn live(&self) -> usize {
        self.live.len()
    }

    fn missile_art(&self, id: u16) -> Option<(Art, &MissileRow)> {
        let row = self.rows.missiles.get(usize::from(id))?;
        if id == 0 || row.cel_file.is_empty() {
            return None;
        }
        let base = format!("data\\global\\missiles\\{}", row.cel_file);
        let art = Art {
            files: [format!("{base}.dcc"), format!("{base}.dc6")],
            rate: if row.anim_rate == 0 {
                256
            } else {
                row.anim_rate
            },
            loops: row.loop_anim,
            offset: unit_offset(None, TableOffset::Missile(Some(row.offset))).unwrap_or((0, 0)),
            // Never the hover target: client missiles are not units here.
            mode: missile_mode(row.trans, false),
            pre_draw: None,
        };
        Some((art, row))
    }

    fn overlay_art(&self, id: u16, looped: bool) -> Option<(Art, &OverlayRow)> {
        let row = self.rows.overlays.get(usize::from(id))?;
        if id == 0 || row.file.is_empty() {
            return None;
        }
        let base = format!("data\\global\\overlays\\{}", row.file);
        let art = Art {
            files: [format!("{base}.dcc"), format!("{base}.dc6")],
            // The rate is `AnimRate` × 16
            // (8.8 frames per tick), not the raw column.
            rate: if row.anim_rate == 0 {
                256
            } else {
                row.anim_rate * 16
            },
            loops: looped,
            offset: row.offset,
            mode: overlay_mode(row.trans),
            pre_draw: Some(row.pre_draw),
        };
        Some((art, row))
    }

    fn fresh_id(&mut self) -> u32 {
        self.next_id = self.next_id.wrapping_add(1);
        TAG_BASE | (self.next_id & 0x00FF_FFFF)
    }

    /// Ticks at the game's rate `at` of one pass through an animation of
    /// `frames` at 8.8 `rate` per base-rate tick (the first tick whose
    /// [`TickRate::progress`] reaches the end).
    fn ticks_of(frames: u32, rate: u32, at: TickRate) -> u64 {
        (u64::from(frames.max(1)) * 256 * u64::from(at.hz()))
            .div_ceil(u64::from(rate.max(1)) * u64::from(TickRate::BASE.hz()))
    }

    /// Starts the effects of every new cast request in the model. The
    /// first call only learns the requests already there.
    fn observe(&mut self, world: &ClientWorld, at: &dyn Fn(&ClientUnit) -> Option<(u32, u32)>) {
        let learn = !self.started;
        self.started = true;
        self.seen.retain(|k, _| world.units.contains_key(k));
        self.seen_hits.retain(|k, _| world.units.contains_key(k));
        let mut casts = Vec::new();
        for unit in world.units.values() {
            // A new hit overlay:
            // created in the message handler, before the update advance of
            // its tick.
            if let Some(h) = unit.last_hit_overlay {
                if self.seen_hits.insert(unit.key, h.count) != Some(h.count) && !learn {
                    let call = HitOverlay {
                        tick: h.tick,
                        unit: unit.key,
                        id: h.id,
                        kind: 2,
                        args: [0; 3],
                        remove: false,
                    };
                    let at = self.late_calls.partition_point(|c| c.tick <= h.tick);
                    self.late_calls.insert(at, call);
                }
            }
            // The unit's last cast, not its last request: the base game starts the
            // cast's effects inside the request,
            // so a later request of the same tick (a 0x13 soft hit) does
            // not cancel them.
            let Some(cast) = unit.last_cast else {
                continue;
            };
            let req = cast.request;
            if !matches!(req.code, CAST_POINT | CAST_UNIT) {
                continue;
            }
            // A repeat of the same request (the same skill at the same
            // target, cast again) is a new cast: the unit's request count
            // tells it from the one already seen.
            if self.seen.insert(unit.key, (req, cast.count)) != Some((req, cast.count)) && !learn {
                casts.push((unit.key, req, cast.late, cast.tick));
            }
        }
        for (key, req, late, tick) in casts {
            // The overlay is born on the request's tick, the caster's mode
            // start, not on the frame that first
            // sees it. A scripted click that reached the model after its
            // update (`ClientWorld::late_input`): the base game created the
            // overlay in that update, one tick earlier. Only the local
            // player's request comes from the script: another unit's comes
            // from a server message and starts on its own tick.
            let late = late && world.local_player == Some(key);
            let born = tick.min(world.server_ticks) - u64::from(late);
            self.start_cast(world, key, req, born, at);
        }
    }

    fn start_cast(
        &mut self,
        world: &ClientWorld,
        key: UnitKey,
        req: ModeRequest,
        born: u64,
        at: &dyn Fn(&ClientUnit) -> Option<(u32, u32)>,
    ) {
        let Some(unit) = world.units.get(&key) else {
            return;
        };
        // The caster where it is drawn (the local player at the frame's
        // one position).
        let Some(origin) = at(unit) else { return };
        let Ok(skill) = usize::try_from(req.record[0]) else {
            return;
        };
        if let Some(&id) = self.rows.skill_overlay.get(skill) {
            if let Some((art, row)) = self.overlay_art(id, false) {
                // None of the id already playing on U:
                // a cast while the last cast's
                // overlay still plays makes none.
                let playing = self.live.iter().any(|f| {
                    f.missile.is_none()
                        && f.follow == Some(key)
                        && f.art.files == art.files
                        && born < f.born + f.life
                }) || self
                    .unit_overlays
                    .get(&key)
                    .is_some_and(|l| l.records.iter().any(|r| r.id == i32::from(id)));
                if playing {
                    return;
                }
                let life = Self::ticks_of(row.frames, art.rate, world.tick_rate);
                let id = self.fresh_id();
                self.live.push(Fx {
                    art,
                    at: origin,
                    follow: Some(key),
                    dir64: 0,
                    born,
                    life,
                    id,
                    missile: None,
                    clt_do_func: 0,
                    anim_len: 0,
                    init_steps: 0,
                    seq: super::overlay::next_seq(),
                });
            }
        }
        // The caster's `cltmissile` is the model's own client missile: the
        // client do on the action frame creates it (`bridge::client_do`;
        // a monster's from its skill step), and [`Self::draws`] draws it.
    }

    /// Runs the effects up to the model's server tick.
    fn advance(&mut self, world: &ClientWorld, log: &mut Vec<String>) {
        let to = world.server_ticks;
        if to < self.now || to - self.now > MAX_CATCH_UP {
            self.run_overlay_calls(world, u64::MAX, log);
            self.run_late_calls(world, u64::MAX, log);
            self.now = to;
            return;
        }
        while self.now < to {
            self.now += 1;
            self.run_overlay_calls(world, self.now, log);
            self.step(world);
            self.run_late_calls(world, self.now, log);
        }
        self.run_overlay_calls(world, to, log);
        self.run_late_calls(world, to, log);
    }

    fn step(&mut self, world: &ClientWorld) {
        let now = self.now;
        let rate = world.tick_rate;
        // Once per unit per update; a list
        // whose host left the model goes with it.
        let rows = &self.rows;
        self.unit_overlays.retain(|key, list| {
            let Some(unit) = world.units.get(key) else {
                return false;
            };
            let mut env = HostEnv::new(rows, unit, &self.overlay_frames);
            list.advance(&mut env, now as u32, kind6_clock_ms(now as u32, rate), rate);
            true
        });
        // A landed item's ground overlay 70, made
        // by the item update (call: kind 6,
        // a = 8) after the unit's walk, so it first advances in the next
        // update. The bridge drew its seed rolls; the record takes the
        // a = 8 start it kept.
        for (key, unit) in &world.units {
            let Some((tick, start)) = unit.ground_overlay else {
                continue;
            };
            let list = self.unit_overlays.entry(*key).or_default();
            if tick > now
                || list
                    .records
                    .iter()
                    .any(|r| r.id == i32::from(GROUND_OVERLAY))
            {
                continue;
            }
            // The model's copy of the seed misses the draw-phase steps
            // (cursor, weather, sound): the start is drawn again on the
            // shared seed as it stood at the create.
            let real = self.client_seed.as_ref().zip(unit.ground_overlay_seed);
            let start = real
                .and_then(|(c, m)| c.lock().unwrap_or_else(|e| e.into_inner()).real_at(m))
                .map_or(start, |mut s| {
                    s.step();
                    s.roll(8 * 256) as i32
                });
            let mut env = HostEnv::new(rows, unit, &self.overlay_frames);
            env.rolls = vec![start, 0];
            list.create(&mut env, i32::from(GROUND_OVERLAY), 6, 8, 0, 0, 0, 0);
        }
        self.live.retain(|fx| now.saturating_sub(fx.born) < fx.life);
    }

    /// Makes one file resident (once per file; a failure is logged once
    /// and stays `None`).
    fn resident(
        &mut self,
        files: &[String; 2],
        assets: &mut ViewAssets,
        log: &mut Vec<String>,
    ) -> Option<&Resident> {
        if !self.files.contains_key(&files[0]) {
            let source = self.source.clone()?;
            let loaded = load(source.as_ref(), files, assets);
            if let Err(e) = &loaded {
                log.push(format!("effect art: {}: {e}", files[0]));
            }
            self.files.insert(files[0].clone(), loaded.ok());
        }
        self.files.get(&files[0])?.as_ref()
    }

    /// Makes the files of the queued creates and of their follow-ups
    /// (kinds 4 / 9: A, B), and the ground overlay of a landed
    /// item, resident and keeps their frame counts
    /// (one that does not load gives 1,
    /// `None` here).
    fn load_overlay_art(
        &mut self,
        world: &ClientWorld,
        assets: &mut ViewAssets,
        log: &mut Vec<String>,
    ) {
        let mut ids: Vec<u16> = Vec::new();
        for c in self
            .overlay_calls
            .iter()
            .chain(&self.late_calls)
            .filter(|c| !c.remove)
        {
            ids.push(c.id);
            if matches!(c.kind, 4 | 9) {
                ids.extend(c.args[..2].iter().filter_map(|&a| u16::try_from(a).ok()));
            }
        }
        if world.units.values().any(|u| u.ground_overlay.is_some()) {
            ids.push(GROUND_OVERLAY);
        }
        ids.retain(|id| !self.overlay_frames.contains_key(id));
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            let frames = match self.overlay_art(id, true) {
                Some((art, _)) => self
                    .resident(&art.files, assets, log)
                    .and_then(|r| i32::try_from(r.frames).ok()),
                None => None,
            };
            self.overlay_frames.insert(id, frames);
        }
    }

    /// Runs the queued UI overlay calls delivered at or before tick `upto`,
    /// in order, on the model's units; a
    /// call on a unit the model no longer has does nothing.
    fn run_overlay_calls(&mut self, world: &ClientWorld, upto: u64, log: &mut Vec<String>) {
        let n = self.overlay_calls.partition_point(|c| c.tick <= upto);
        for call in self.overlay_calls.drain(..n).collect::<Vec<_>>() {
            self.run_call(world, call, log);
        }
    }

    /// Runs the queued hit overlay creates of tick `upto` or earlier, in
    /// order (kind 2).
    fn run_late_calls(&mut self, world: &ClientWorld, upto: u64, log: &mut Vec<String>) {
        let n = self.late_calls.partition_point(|c| c.tick <= upto);
        for call in self.late_calls.drain(..n).collect::<Vec<_>>() {
            self.run_call(world, call, log);
        }
    }

    fn run_call(&mut self, world: &ClientWorld, call: HitOverlay, log: &mut Vec<String>) {
        let Some(unit) = world.units.get(&call.unit) else {
            return;
        };
        let id = i32::from(call.id);
        let mut env = HostEnv::new(&self.rows, unit, &self.overlay_frames);
        if !call.remove {
            // a = b = 0 for every caller (no seed draw); the state
            // overlays' ids carry their `1ofN` roll.
            let [arg_a, arg_b, arg_c] = call.args;
            self.unit_overlays
                .entry(call.unit)
                .or_default()
                .create(&mut env, id, call.kind, 0, arg_a, arg_b, arg_c, 0);
        } else if let Some(list) = self.unit_overlays.get_mut(&call.unit) {
            if let Err(e) = list.remove_by_id(&mut env, id) {
                log.push(format!("overlay remove {id}: {e:?}"));
            }
        }
    }

    /// Makes the art of the live effects and of the units' state overlays
    /// resident, once per file. One log line per new failure.
    fn ensure(&mut self, world: &ClientWorld, assets: &mut ViewAssets) -> Vec<String> {
        let mut log = Vec::new();
        let Some(source) = self.source.clone() else {
            return log;
        };
        let mut wanted: Vec<[String; 2]> = self.live.iter().map(|f| f.art.files.clone()).collect();
        for (_, unit) in bridge_missiles(world) {
            if let Some((art, _)) = u16::try_from(unit.class)
                .ok()
                .and_then(|c| self.missile_art(c))
            {
                wanted.push(art.files);
            }
        }
        for files in wanted {
            if self.files.contains_key(&files[0]) {
                continue;
            }
            let loaded = load(source.as_ref(), &files, assets);
            if let Err(e) = &loaded {
                log.push(format!("effect art: {}: {e}", files[0]));
            }
            self.files.insert(files[0].clone(), loaded.ok());
        }
        log
    }

    fn draw_one(&self, camera: &Camera, assets: &ViewAssets, p: Placing<'_>) -> Option<DrawItem> {
        let Placing {
            art,
            at,
            dir64,
            progress,
            life,
            tag,
            frame: fixed,
            light,
            motion,
        } = p;
        let offset = (
            art.offset.0.wrapping_add(motion.0),
            art.offset.1.wrapping_add(motion.1),
        );
        let Some(Some(res)) = self.files.get(&art.files[0]) else {
            return None;
        };
        let frame = (progress >> 8) as usize;
        let frame = match fixed {
            Some(f) if f < res.frames => f,
            Some(_) => return None,
            None if art.loops || life == u64::MAX => frame % res.frames.max(1),
            None => frame.min(res.frames.saturating_sub(1)),
        };
        let dir = if res.directions > 1 {
            file_direction(res.directions, dir64).unwrap_or(0)
        } else {
            0
        };
        let set = FrameSetKey::new(res.path.as_str(), FramePart::Dir(dir)).ok()?;
        let (id, image) = (
            assets.id(&set, frame).ok()?,
            assets.frame(&set, frame).ok()?,
        );
        let (x, y) = camera.unit_draw(moving_to_client(at.0, at.1), offset);
        let placed = place(image, x, y, Rect::world_clip());
        let clip = placed.clip?;
        // No remap (overlays drop the blood map, Edge
        // case 5; a missile's blood map needs the green-blood switch, off
        // on the reference install, and the client
        // missiles have no unit palette index); light byte `light`: 0xFF
        // (unlit) for missiles, the host's for overlays.
        let (shade, blend) = match &assets.shades {
            Some(t) => cel_ops(t, art.mode, None, light),
            None if art.mode == MODE_OPAQUE => (ShadeChain::EMPTY, BlendOp::Opaque),
            None => return None,
        };
        let mut d = DrawItem::new(id, placed.x, placed.y);
        d.clip = clip;
        d.shade = shade;
        d.blend = blend;
        // The art's mode, no palette.
        d.call = Some(crate::scene::CelArgs {
            mode: art.mode,
            pal: crate::scene::CelPal::Null,
        });
        d.tag = ItemTag::Unit(tag);
        Some(d)
    }

    /// The draws of the live effects and the state overlays whose art is
    /// resident, under `camera`, in creation order; a unit's overlays at
    /// `at(unit)`, the 16.16 position the unit is drawn at. A missile
    /// for which `hidden(missile row, 16.16 position)` is true is not
    /// drawn (the sight test). A unit's overlays
    /// take `host_light(unit)`, the light byte its unit draw hands the
    /// overlay draw.
    pub fn draws(
        &self,
        world: &ClientWorld,
        camera: &Camera,
        assets: &ViewAssets,
        at: &dyn Fn(&ClientUnit) -> Option<(u32, u32)>,
        hidden: &dyn Fn(u16, (u32, u32)) -> bool,
        host_light: &dyn Fn(&ClientUnit) -> u8,
    ) -> Vec<MissileDraw> {
        let mut found: Vec<(u32, DrawItem, Join, u64)> = Vec::new();
        for fx in &self.live {
            if fx.missile.is_some_and(|m| hidden(m, fx.at)) {
                continue;
            }
            // `InitSteps`: created not drawn, drawn once the elapsed
            // updates exceed it.
            // `InitSteps` counts base-rate updates.
            let init = world
                .tick_rate
                .ticks(i32::try_from(fx.init_steps).unwrap_or(i32::MAX));
            if fx.init_steps > 0
                && world.server_ticks.saturating_sub(fx.born) <= u64::from(init.unsigned_abs())
            {
                continue;
            }
            let at = fx
                .follow
                .and_then(|k| world.units.get(&k))
                .and_then(at)
                .unwrap_or(fx.at);
            let age = world.server_ticks.saturating_sub(fx.born);
            let p = Placing {
                art: &fx.art,
                at,
                dir64: fx.dir64,
                progress: world.tick_rate.progress(u64::from(fx.art.rate), age),
                life: fx.life,
                // A host-bound effect (a cast overlay) draws inside the
                // host's unit draw: no unit row of its own.
                tag: fx.follow.map_or(fx.id, |k| k.guid),
                frame: None,
                light: UNLIT,
                motion: (0, 0),
            };
            let join = match (fx.art.pre_draw, fx.follow) {
                (Some(back), Some(host)) => Join::Host { host, back },
                _ if fx.flat(age, world.tick_rate) => Join::Flat {
                    at: moving_to_client(at.0, at.1),
                },
                _ => Join::Cell {
                    at: moving_to_client(at.0, at.1),
                },
            };
            if let Some(d) = self.draw_one(camera, assets, p) {
                found.push((fx.id, d, join, fx.seq));
            }
        }
        // The client missiles of the model (the bridge's client do):
        // each draws one cel of its `CelFile` at
        // its path position, direction and 8.8 frame,
        // inside its own unit draw (its room-list slot, after the
        // unit row); flag-ex 0x40000 (`InitSteps`) draws none.
        for (key, unit) in bridge_missiles(world) {
            if unit.flag_ex & FLAG_EX_NOT_DRAWN != 0 {
                continue;
            }
            let Some(m) = world.objclient.missiles.get(&key) else {
                continue;
            };
            let Ok(class) = u16::try_from(unit.class) else {
                continue;
            };
            if hidden(class, m.pos) {
                continue;
            }
            let Some((art, _)) = self.missile_art(class) else {
                continue;
            };
            let Ok(frame) = usize::try_from(m.frame >> 8) else {
                continue;
            };
            let vk = view_key(key);
            // The cel is placed from the path's client pixel pair, which
            // a function 6 / 18 child keeps at its creation centre while
            // its precise point is the parent's (the drawn point); the
            // sight test above reads the precise sub-tile.
            let p = Placing {
                art: &art,
                at: m.drawn_at.unwrap_or(m.pos),
                dir64: m.direction,
                progress: 0,
                life: u64::MAX,
                tag: vk.guid,
                frame: Some(frame),
                // The missile's own unit light byte, as
                // any unit draw (measured: the blood splats are
                // `L[P[s]]`).
                light: host_light(unit),
                // The unit draw adds the motion record's offsets (a
                // lobbed missile's arc height, `oz`).
                motion: m.motion.draw_offset(),
            };
            if let Some(d) = self.draw_one(camera, assets, p) {
                found.push((
                    vk.guid,
                    d,
                    Join::Host {
                        host: vk,
                        back: false,
                    },
                    0,
                ));
            }
        }
        // The records of the UI's overlay
        // calls, in list order (newest first), each in its host's back or
        // front call. The item carries the host's tag: the overlay draw
        // runs inside the host's unit draw. The cel direction
        // is file direction 0 (the UI's overlay, `npcalert`, has one).
        for (key, list) in &self.unit_overlays {
            let Some(unit) = world.units.get(key) else {
                continue;
            };
            let Some(unit_at) = at(unit) else { continue };
            for (i, rec) in list.records.iter().enumerate() {
                let back = rec.pre_draw;
                if !overlay_draws(rec, back) {
                    continue;
                }
                let (Ok(oid), Ok(f)) = (u16::try_from(rec.id), usize::try_from(rec.frame_index()))
                else {
                    continue;
                };
                let Some((mut art, _)) = self.overlay_art(oid, true) else {
                    continue;
                };
                // `Xoffset`, `Yoffset` plus the host's height.
                art.offset = (rec.x, rec.y);
                let p = Placing {
                    art: &art,
                    at: unit_at,
                    dir64: 0,
                    progress: 0,
                    life: u64::MAX,
                    tag: key.guid,
                    frame: Some(f),
                    light: host_light(unit),
                    motion: (0, 0),
                };
                if let Some(d) = self.draw_one(camera, assets, p) {
                    let id = TAG_BASE | 0x0040_0000 | (key.guid & 0xFFFF) << 4 | (i as u32 & 0xF);
                    found.push((id, d, Join::Host { host: *key, back }, rec.seq));
                }
            }
        }
        // One unit's overlays draw newest first, whichever list holds
        // them (the base game's one list, head push):
        // a cast overlay of this layer and a hit overlay record of the
        // same host (measured: `firehit` before
        // the older `holyboltcloud`). Other joins keep their build order.
        found.sort_by_key(|&(_, _, join, seq)| match join {
            Join::Host { .. } => std::cmp::Reverse(seq),
            _ => std::cmp::Reverse(u64::MAX),
        });
        found
            .into_iter()
            .enumerate()
            .filter_map(|(minor, (id, mut item, join, _))| {
                let minor = u32::try_from(minor).ok()?;
                item.key = DrawKey::new(pass::WALLS_UNITS, DrawKey::MAJOR_MAX, minor, 0).ok()?;
                Some(MissileDraw { id, item, join })
            })
            .collect()
    }

    /// The draws keyed into the frame's draw order (`slots`, the units'
    /// slots), each with whether it goes before the items of equal key
    /// (`true`) or after them:
    /// - an overlay takes its host's slot, sub 0 before the host's items
    ///   when it is a back overlay, else sub 255 after them; a host the
    ///   order does not draw hides it;
    /// - a missile is filed in the unit list of its cell (,
    ///   the cell of its client position in `grid`): after the
    ///   cell's walls, before the first unit of the cell whose client y
    ///   (`y_of`) is not below its own (the list's Y sort;
    ///   prepended at creation, so ahead on equal y), else after
    ///   the cell's units; outside the grid it is not filed (not drawn).
    pub fn keyed(
        draws: &[MissileDraw],
        slots: &BTreeMap<UnitKey, UnitSlot>,
        grid: &DrawGrid,
        y_of: impl Fn(UnitKey) -> Option<i32>,
    ) -> Vec<(MissileDraw, bool)> {
        let mut out = Vec::with_capacity(draws.len());
        for d in draws {
            let (key, before) = match d.join {
                Join::Host { host, back } => {
                    let Some(UnitSlot::Drawn(k)) = slots.get(&host).copied() else {
                        continue;
                    };
                    let sub = if back { 0 } else { u8::MAX };
                    (DrawKey::new(k.pass, k.major, k.minor, sub), back)
                }
                Join::Flat { at } => {
                    let Some(ci) = grid.cell(tile_of(at.x, at.y)) else {
                        continue;
                    };
                    // D2RUST's own, unverified: the effect is in no room unit
                    // list, so it appends after the cell's shadow entries.
                    let key = DrawKey::new(pass::SHADOWS, ci as u32, DrawKey::MINOR_MAX, u8::MAX);
                    (key, false)
                }
                Join::Cell { at } => {
                    let Some(ci) = grid.cell(tile_of(at.x, at.y)) else {
                        continue;
                    };
                    let ci = ci as u32;
                    let mut units: Vec<(u32, UnitKey)> = slots
                        .iter()
                        .filter_map(|(&key, slot)| match slot {
                            UnitSlot::Drawn(k) if k.pass == pass::WALLS_UNITS && k.major == ci => {
                                Some((k.minor, key))
                            }
                            _ => None,
                        })
                        .collect();
                    units.sort();
                    let next = units
                        .iter()
                        .find(|&&(_, key)| y_of(key).is_some_and(|y| y >= at.y));
                    match next {
                        Some(&(minor, _)) => (DrawKey::new(pass::WALLS_UNITS, ci, minor, 0), true),
                        None => (
                            DrawKey::new(pass::WALLS_UNITS, ci, DrawKey::MINOR_MAX, u8::MAX),
                            false,
                        ),
                    }
                }
            };
            let Ok(key) = key else { continue };
            let mut d = *d;
            d.item.key = key;
            out.push((d, before));
        }
        out
    }

    /// Runs the layer for this frame and adds its draws to a built
    /// `frame` (re-sorted by key), under the frame's one camera
    /// (`frame.camera`), each unit at
    /// the position `feed` draws it at (the local player at its predicted
    /// position). No camera, or open mode 3 (no world):
    /// nothing. Never fails the frame; returns log lines.
    pub fn add_to_frame<F: ViewFeed + ?Sized>(
        &mut self,
        world: &ClientWorld,
        feed: &F,
        assets: &mut ViewAssets,
        frame: &mut WorldFrame,
    ) -> Vec<String> {
        self.last.clear();
        if self.rows.is_empty() {
            self.overlay_calls.clear();
            self.late_calls.clear();
            return Vec::new();
        }
        let at = |u: &ClientUnit| unit_at(feed, u);
        self.observe(world, &at);
        let mut log = Vec::new();
        self.load_overlay_art(world, assets, &mut log);
        self.advance(world, &mut log);
        log.extend(self.ensure(world, assets));
        let camera = match (feed.player(world), feed.open_mode(world)) {
            // The frame's one camera, shake included;
            // a frame built without one:
            // the player's, unshaken.
            (Ok(Some(p)), Ok(mode)) if !super::feed::no_world(mode) => frame
                .camera
                .unwrap_or_else(|| Camera::new(FrameSize::play(), mode, p.client(), (0, 0))),
            (Err(e), _) | (_, Err(e)) => {
                log.push(format!("effects: no camera: {e}"));
                return log;
            }
            _ => return log,
        };
        let hidden = |missile: u16, pos: (u32, u32)| missile_hidden(world, feed, missile, pos);
        // An overlay draws with the light byte
        // of its host's unit draw, doubled and
        // clamped when the host is drawn highlighted;
        // no frame light: unlit.
        let light = match feed.light(world) {
            Ok(l) => l,
            Err(e) => {
                log.push(format!("effects: no frame light: {e}"));
                None
            }
        };
        let host_light = |u: &ClientUnit| match light {
            Some(l) => crate::rules::lighting::view::unit_light_byte(l.light, l.look, u),
            None => UNLIT,
        };
        self.last = self.draws(world, &camera, assets, &at, &hidden, &host_light);
        // A host whose body fails the COF
        // box pre-test draws none of its overlays (the composite returns
        // first; measured: Warriv's `npcalert` stops with his body).
        self.last.retain(
            |d| !matches!(d.join, Join::Host { host, .. } if frame.body_culled.contains(&host)),
        );
        let Some(slots) = &frame.slots else {
            // No draw order (no map feed): after every other unit.
            if !self.last.is_empty() {
                frame.items.extend(self.last.iter().map(|d| d.item));
                crate::scene::order(&mut frame.items);
            }
            return log;
        };
        let y_of = |k: UnitKey| {
            let u = world.units.get(&k)?;
            feed.unit_position(u).ok().map(|p| p.client().y)
        };
        // The overlay draw runs in the
        // composite's slot loop, after the COF box pre-test; a host that
        // fails it (its unit call alone, no cel) draws no overlay. A
        // missile has no pre-test (one cel, no COF); its unit
        // call carries no path, and its cel joins that call as host.
        let culled: std::collections::BTreeSet<u64> = frame
            .unit_calls
            .iter()
            .filter(|c| c.path.is_none() && !c.shadow)
            .map(|c| c.key.slot())
            .collect();
        self.last.retain(
            |d| !matches!(d.join, Join::Host { host, .. } if host_culled(host, slots, &culled)),
        );
        let keyed = Self::keyed(&self.last, slots, &DrawGrid::of_camera(&camera), y_of);
        // Into the sorted list: the "after" draws in order, then the
        // "before" draws in reverse order, so equal keys keep build order.
        for (d, _) in keyed.iter().filter(|(_, before)| !before) {
            let k = d.item.key;
            let at = frame.items.partition_point(|i| i.key <= k);
            frame.items.insert(at, d.item);
        }
        for (d, _) in keyed.iter().rev().filter(|(_, before)| *before) {
            let k = d.item.key;
            let at = frame.items.partition_point(|i| i.key < k);
            frame.items.insert(at, d.item);
        }
        // A client missile's cel row reads its unit
        // draw's `dir64`, the missile's direction (not the file direction).
        for (d, _) in &keyed {
            if let Join::Host { host, .. } = d.join {
                // Between ticks the draw moves with its host (a client
                // missile is its own host): `unit_motion`.
                if super::unit_motion::moves(host) {
                    frame.movers.entry(d.item.key.slot()).or_insert(host);
                }
                if host.unit_type != MISSILE || host.guid & VIEW_CLIENT_BIT == 0 {
                    continue;
                }
                let real = UnitKey::new(host.unit_type, host.guid & !VIEW_CLIENT_BIT);
                if let Some(m) = world.objclient.missiles.get(&real) {
                    frame.unit_dirs.insert(d.item.key.slot(), m.direction);
                }
            }
        }
        self.last = keyed.into_iter().map(|(d, _)| d).collect();
        log
    }
}

/// Whether `host` failed the COF box pre-test:
/// its slot's unit call drew no cel (`culled`). Only types 0–2 take the
/// composite path with that pre-test; a missile or an item draws one cel
/// with none, and a client missile's unit call never carries a
/// cel path (this layer draws its cel), so it is never culled here.
fn host_culled(
    host: UnitKey,
    slots: &BTreeMap<UnitKey, UnitSlot>,
    culled: &std::collections::BTreeSet<u64>,
) -> bool {
    if host.unit_type >= MISSILE {
        return false;
    }
    matches!(
        slots.get(&host),
        Some(UnitSlot::Drawn(k)) if DrawKey::new(k.pass, k.major, k.minor, 0)
            .is_ok_and(|d| culled.contains(&d.slot()))
    )
}

/// The 16.16 position `feed` draws `unit` at: moving
/// units as stated (the local player at its predicted position), static
/// units and unresolved ones at their model sub-tile centre.
/// What the overlay rules read from a host unit:
/// the rows, the unit, and the frame count of the file being created.
struct HostEnv<'a> {
    rows: &'a EffectRows,
    unit: &'a ClientUnit,
    /// The file frame count by overlay id.
    frames: &'a BTreeMap<u16, Option<i32>>,
    /// The state the kind-8 cycle has cleared for its aura test.
    off: Option<i32>,
    /// The seed draws the bridge already made for this create, last
    /// first (the item's ground overlay); empty: every roll gives 0.
    rolls: Vec<i32>,
}

impl<'a> HostEnv<'a> {
    fn new(
        rows: &'a EffectRows,
        unit: &'a ClientUnit,
        frames: &'a BTreeMap<u16, Option<i32>>,
    ) -> Self {
        HostEnv {
            rows,
            unit,
            frames,
            off: None,
            rolls: Vec::new(),
        }
    }

    fn monster(&self) -> Option<(bool, Option<u8>)> {
        if self.unit.key.unit_type != MONSTER {
            return None;
        }
        let class = usize::try_from(self.unit.class).ok()?;
        Some(
            self.rows
                .monster_overlay
                .get(class)
                .copied()
                .unwrap_or((false, None)),
        )
    }
}

impl OverlayEnv for HostEnv<'_> {
    fn expansion(&self) -> bool {
        !self.rows.classic_install
    }
    fn row_count(&self) -> i32 {
        i32::try_from(self.rows.overlay_rules.len()).unwrap_or(i32::MAX)
    }
    fn row(&self, id: i32) -> Option<OverlayRules> {
        self.rows
            .overlay_rules
            .get(usize::try_from(id).ok()?)
            .copied()
    }
    fn unit_blocks_overlays(&self) -> bool {
        self.unit.flag_ex & 0x40000 != 0
    }
    fn unit_no_ovly(&self) -> bool {
        self.monster().is_some_and(|m| m.0)
    }
    fn height_unit(&self) -> HeightUnit {
        match (self.unit.key.unit_type, self.monster()) {
            (PLAYER, _) => HeightUnit::Player,
            (_, Some((_, overlay_height))) => HeightUnit::Monster { overlay_height },
            _ => HeightUnit::Other,
        }
    }
    fn frame_count(&self, id: i32) -> Option<i32> {
        self.frames.get(&u16::try_from(id).ok()?).copied().flatten()
    }
    /// The model's creates pass a = b = 0 and make no seed draw (the
    /// state overlays roll their `1ofN` in the bridge); the item's ground
    /// overlay replays the bridge's draws.
    fn roll(&mut self, _n: i32) -> i32 {
        self.rolls.pop().unwrap_or(0)
    }
    fn mode(&self) -> i32 {
        self.unit.mode as i32
    }
    fn has_state(&self, s: i32) -> bool {
        self.off != Some(s) && u8::try_from(s).is_ok_and(|s| self.unit.states.contains(&s))
    }
    /// The kind-8 cycle's bit clear and set around its aura test:
    /// the model's bits stay as they are, the test sees the bit cleared.
    fn set_state(&mut self, s: i32, on: bool) {
        if on {
            if self.off == Some(s) {
                self.off = None;
            }
        } else if self.has_state(s) {
            self.off = Some(s);
        }
    }
    /// With (U): U has an `aura` state.
    fn has_aura(&self) -> bool {
        self.unit
            .states
            .iter()
            .any(|&s| self.off != Some(i32::from(s)) && self.rows.aura_states.contains(&s))
    }
}

/// The model's client missiles (set C units of type 3 with a missile
/// record), in key order.
fn bridge_missiles(world: &ClientWorld) -> impl Iterator<Item = (UnitKey, &ClientUnit)> {
    world
        .objclient
        .set_c
        .iter()
        .filter(|(k, _)| k.unit_type == MISSILE && world.objclient.missiles.contains_key(k))
        .map(|(k, u)| (*k, u))
}

fn unit_at<F: ViewFeed + ?Sized>(feed: &F, unit: &ClientUnit) -> Option<(u32, u32)> {
    match feed.unit_position(unit) {
        Ok(UnitPosition::Moving { x16, y16 }) => Some((x16, y16)),
        // A static unit (types 2, 4, 5) stands on its sub-tile's corner,
        // the point its own draw uses, not the centre.
        Ok(UnitPosition::Static { sx, sy }) => {
            Some((u32::try_from(sx).ok()? << 16, u32::try_from(sy).ok()? << 16))
        }
        _ => unit.position.map(cell_centre),
    }
}

/// The sight test of a missile (missiles are
/// tested): the feed's answer for a missile unit
/// of row `missile` on the sub-tile of its 16.16 position (the dynamic
/// path's current sub-tile; size `missiles` `Size`). Hidden only on a
/// `Some(true)` answer; a feed
/// that cannot run the test draws it.
///
/// `D2RUST's own, unverified`: the effect layer's missiles are not model
/// units (no client missile creation), so the probe unit stands for one.
pub(crate) fn missile_hidden<F: ViewFeed + ?Sized>(
    world: &ClientWorld,
    feed: &F,
    missile: u16,
    pos: (u32, u32),
) -> bool {
    let (Ok(x), Ok(y)) = (u16::try_from(pos.0 >> 16), u16::try_from(pos.1 >> 16)) else {
        return false;
    };
    let mut probe = ClientUnit::new(UnitKey::new(MISSILE, u32::MAX));
    probe.class = u32::from(missile);
    probe.position = Some((x, y));
    feed.unit_facts(world, &probe)
        .is_ok_and(|f| f.sight_hidden == Some(true))
}

/// Reads the first of `files` found into the frame store, every
/// direction.
fn load(
    source: &dyn FileSource,
    files: &[String; 2],
    assets: &mut ViewAssets,
) -> Result<Resident, String> {
    let (dcc, dc6) = (&files[0], &files[1]);
    let path = CanonicalPath::new(dcc).map_err(|e| e.to_string())?;
    let mut sets = Vec::new();
    let frames;
    if let Some(r) = crate::assets::path::read_dcc(source, dcc) {
        let d = r?;
        let n = u8::try_from(d.directions.len()).map_err(|_| "too many directions")?;
        for dir in 0..n {
            sets.push(FrameSet::from_dcc(&d, dir).map_err(|e| e.to_string())?);
        }
        frames = d.frames_per_direction as usize;
    } else if let Some(r) = crate::assets::path::read_dc6(source, dc6) {
        let d = r?;
        let n = u8::try_from(d.header.directions).map_err(|_| "too many directions")?;
        for dir in 0..n {
            sets.push(FrameSet::from_dc6(&d, dir).map_err(|e| e.to_string())?);
        }
        frames = d.header.frames_per_direction as usize;
    } else {
        return Err("in no archive".into());
    }
    // Both formats are filed under the `.dcc` name: one key per file.
    let directions = sets.len() as u8;
    for (dir, set) in sets.into_iter().enumerate() {
        let key = FrameSetKey::new(path.as_str(), FramePart::Dir(dir as u8))
            .map_err(|e| e.to_string())?;
        if !assets.frames.contains(&key) {
            assets.frames.insert(key, set).map_err(|e| e.to_string())?;
        }
    }
    Ok(Resident {
        path,
        directions,
        frames,
    })
}

#[cfg(test)]
#[path = "missiles_tests.rs"]
mod tests;
