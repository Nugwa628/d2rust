//! Units and missiles between ticks: players, monsters and missiles glide
//! from where one drawn tick put them to where the next one did, at
//! display rate, while the game logic and the game frame stay at the tick
//! rate.
//!
//! The motion comes only from the simulation's own positions: at every
//! drawn tick the world view notes each moving unit's fixed-point position
//! as the client keeps it (the 16.16 sub-tile position of a moving unit,
//! or the 16.16 point a client missile is drawn at), and [`UnitMotion`]
//! keeps the previous and the current one. Between ticks a unit is drawn
//! at the blend of the two, taken in fixed point and turned into client
//! pixels like any position. Nothing is guessed from rendered frames and
//! nothing runs ahead of the last tick.
//!
//! Like the smooth camera (`super::smooth`) the blend is one tick behind:
//! right after tick N is drawn a unit stands where tick N−1 put it, and it
//! reaches tick N's position when the next tick is due. That holds on
//! every display frame, the ones that bring a new tick included, so a unit
//! only ever moves forward ([`frame_motion`]). The camera's blend
//! fraction is shared, so the ground and the units move in step.
//!
//! Units blend only while the display shows clearly more frames than
//! there are ticks ([`super::smooth::BLEND_ON`]). At a lower display rate
//! nearly every frame brings a new tick, and units are drawn where the
//! tick put them, with no compose of their own.
//!
//! On a display frame between ticks the tick frame's draw list is copied,
//! every item of a moving unit (its body, shadow and overlays, found by
//! their draw slot, [`super::WorldFrame::movers`]) is moved by the part of
//! the unit's step not shown yet, and the copy is composed into the
//! presentation layers only ([`shifted_items`]). That compose never
//! commits: the persistent game frame and the tick frames stay exactly as
//! they were. It runs at the game frame's size, so its cost does not grow
//! with the screen resolution.
//!
//! A unit snaps (draws at its tick position) when it has no previous
//! position (it just appeared), when a placement moved it (a teleport,
//! [`crate::bridge::ClientUnit::placements`]), when it entered or left the
//! map, when its step is longer than [`super::smooth::SNAP_PX`] on either
//! axis, and when ticks were missed. A room crossing while walking is a
//! step like any other.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bevy::math::IVec2;
use bevy::prelude::*;

use crate::bridge::world::{ClientWorld, UnitKey, MISSILE, MONSTER, PLAYER, VIEW_CLIENT_BIT};
use crate::gpu_compositor::pack::LAYER_WORLD;
use crate::gpu_compositor::Packed;
use crate::rules::camera::{moving_to_client, ClientPos, UnitPosition};
use crate::scene::DrawItem;

use super::feed::ViewFeed;
use super::node::MotionJob;
use super::present::{WorldViewGpu, WorldViewState};
use super::smooth::{SmoothCamera, SmoothLayers, MAX_TICK_GAP, SNAP_PX};

/// Whether units of `key`'s type move between ticks: players, monsters
/// and missiles. Objects, items and tiles stand still.
pub fn moves(key: UnitKey) -> bool {
    matches!(key.unit_type, PLAYER | MONSTER | MISSILE)
}

/// Where a drawn tick showed a moving unit, as the world view noted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    /// The unit (its view key: a client-made unit has the client bit).
    pub key: UnitKey,
    /// Its position: 16.16 fixed sub-tiles, from the simulation.
    pub pos: (u32, u32),
    /// The unit's placement count: a change is a teleport.
    pub placements: u32,
    /// Whether the unit is on the map (in a room).
    pub on_map: bool,
}

/// One unit's last two drawn positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Track {
    /// The 16.16 position at the previous drawn tick, when the step from
    /// it is blended; `None`: the unit snaps.
    prev: Option<(u32, u32)>,
    seen: Seen,
}

/// The previous and current position of every moving unit of the last
/// two drawn ticks.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct UnitMotion {
    /// The server tick of the last drawn frame.
    tick: Option<u64>,
    tracks: BTreeMap<UnitKey, Track>,
}

impl UnitMotion {
    /// Notes the moving units of the drawn frame of `tick`. A tick seen
    /// again changes nothing. Units not seen this tick are forgotten.
    pub fn observe(&mut self, tick: u64, seen: impl IntoIterator<Item = Seen>) {
        if self.tick == Some(tick) {
            return;
        }
        let gap = self.tick.map_or(0, |t| tick.wrapping_sub(t));
        let blends = (1..=MAX_TICK_GAP).contains(&gap);
        let mut tracks = BTreeMap::new();
        for s in seen {
            let prev = self
                .tracks
                .get(&s.key)
                .filter(|_| blends)
                .and_then(|old| step_from(&old.seen, &s));
            tracks.insert(s.key, Track { prev, seen: s });
        }
        self.tracks = tracks;
        self.tick = Some(tick);
    }

    /// How far `key`'s items move from where the tick frame drew them, at
    /// blend fraction `alpha` (0 right after the tick, 1 at the next):
    /// the client pixels of the blended 16.16 position less those of the
    /// tick's, so in whole frame pixels. Zero for a unit that snaps or is
    /// not tracked.
    pub fn shift(&self, key: UnitKey, alpha: f32) -> IVec2 {
        let Some(t) = self.tracks.get(&key) else {
            return IVec2::ZERO;
        };
        let Some(prev) = t.prev else {
            return IVec2::ZERO;
        };
        let at = blend(prev, t.seen.pos, alpha);
        let (from, to) = (client(at), client(t.seen.pos));
        IVec2::new(from.x - to.x, from.y - to.y)
    }

    /// Whether any tracked unit has a step to blend.
    pub fn moving(&self) -> bool {
        self.tracks.values().any(|t| t.prev.is_some())
    }
}

/// The client pixels of a 16.16 position.
fn client((x, y): (u32, u32)) -> ClientPos {
    moving_to_client(x, y)
}

/// The 16.16 position at blend fraction `alpha` from `prev` (0) to `cur`
/// (1), rounded to the nearest fixed-point step.
fn blend(prev: (u32, u32), cur: (u32, u32), alpha: f32) -> (u32, u32) {
    let rest = f64::from(1.0 - alpha.clamp(0.0, 1.0));
    let mix = |p: u32, c: u32| {
        let d = (i64::from(p) - i64::from(c)) as f64 * rest;
        (i64::from(c) + d.round() as i64) as u32
    };
    (mix(prev.0, cur.0), mix(prev.1, cur.1))
}

/// The previous position `old` when the step to `new` blends, else `None`.
fn step_from(old: &Seen, new: &Seen) -> Option<(u32, u32)> {
    if old.placements != new.placements || !old.on_map || !new.on_map {
        return None;
    }
    let (a, b) = (client(old.pos), client(new.pos));
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let still = old.pos == new.pos;
    (dx.abs() <= SNAP_PX && dy.abs() <= SNAP_PX && !still).then_some(old.pos)
}

/// Where the drawn frame showed each of its moving units (the distinct
/// units of `movers`), from the simulation's fixed-point positions as the
/// client keeps them: a client missile's drawn 16.16 point, else the
/// unit's position from the feed (the 16.16 position of a moving unit). A
/// unit the world or the feed no longer places is left out (it snaps).
pub fn seen_of(
    world: &ClientWorld,
    feed: &(impl ViewFeed + ?Sized),
    movers: &BTreeMap<u64, UnitKey>,
) -> Vec<Seen> {
    let keys: BTreeSet<UnitKey> = movers.values().copied().collect();
    let mut out = Vec::with_capacity(keys.len());
    for key in keys {
        let client_made = key.guid & VIEW_CLIENT_BIT != 0;
        if key.unit_type == MISSILE && client_made {
            let real = UnitKey::new(key.unit_type, key.guid & !VIEW_CLIENT_BIT);
            if let Some(m) = world.objclient.missiles.get(&real) {
                out.push(Seen {
                    key,
                    pos: m.drawn_at.unwrap_or(m.pos),
                    placements: 0,
                    on_map: true,
                });
            }
            continue;
        }
        let Some(unit) = world.view_unit(&key) else {
            continue;
        };
        let Ok(at) = feed.unit_position(&unit) else {
            continue;
        };
        let pos = match at {
            UnitPosition::Moving { x16, y16 } => (x16, y16),
            // Whole sub-tiles, as 16.16.
            UnitPosition::Static { sx, sy } => ((sx as u32) << 16, (sy as u32) << 16),
        };
        out.push(Seen {
            key,
            pos,
            placements: unit.placements,
            on_map: client_made || world.room_units.room_of(key).is_some(),
        });
    }
    out
}

/// The last tick frame handed to the GPU, kept for the display frames
/// until the next one: its draw list, its packed form (size, clears and
/// which items are on the screen layer) and its moving units.
#[derive(Resource, Clone)]
pub struct TickFrame {
    /// The server tick it shows.
    pub tick: u64,
    /// Its [`super::node::ComposeJob::seq`].
    pub seq: u64,
    /// Its `pages_version`.
    pub pages_version: u64,
    pub items: Arc<Vec<DrawItem>>,
    pub packed: Arc<Packed>,
    /// [`super::WorldFrame::movers`].
    pub movers: BTreeMap<u64, UnitKey>,
    /// Its moving units' positions ([`seen_of`]).
    pub seen: Vec<Seen>,
}

/// Whether the presentation layers show units between two ticks this
/// display frame (moved from where the tick frame drew them).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MotionShown(pub bool);

/// What the layers were last composed with: the tick job's `seq` and
/// every moving unit's shift.
type Composed = (u64, Vec<(UnitKey, IVec2)>);

/// Where a display frame at `now` draws the moving units: `None` where the
/// tick put them (the tick frame itself is presented), or `Some(alpha)`,
/// the blend fraction between their previous and current tick positions.
/// While units blend this is the same on every frame, a tick frame
/// included (alpha 0 right at the tick: the units one tick behind), so a
/// unit never steps back from the tick position to the blend.
pub fn frame_motion(smooth: &SmoothCamera, now: std::time::Duration) -> Option<f32> {
    smooth.units_blend().then(|| smooth.alpha(now))
}

/// On every display frame: notes a new tick frame's units, and while
/// units blend ([`frame_motion`]) hands the node a [`MotionJob`] whenever a
/// unit's whole-pixel shift changed since the layers were last composed.
/// The rounding bounds the work: a unit stepping `n` frame pixels per tick
/// asks for at most `n + 1` composes in that tick, whatever the display
/// rate.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compose_motion(
    mut commands: Commands,
    mut motion: ResMut<UnitMotion>,
    mut shown: ResMut<MotionShown>,
    smooth: Res<SmoothCamera>,
    tick: Option<Res<TickFrame>>,
    gpu: Option<Res<WorldViewGpu>>,
    state: Res<WorldViewState>,
    layers: Option<Res<SmoothLayers>>,
    time: Res<Time<Real>>,
    mut composed: Local<Option<Composed>>,
    mut seq: Local<u64>,
) -> Result {
    let (Some(tick), Some(gpu), Some(layers)) = (tick, gpu, layers) else {
        shown.set_if_neq(MotionShown(false));
        return Ok(());
    };
    if tick.is_changed() {
        // The new tick frame's job writes the layers with every unit
        // where the tick put it.
        motion.observe(tick.tick, tick.seen.iter().copied());
        *composed = Some((tick.seq, Vec::new()));
    }
    let Some(alpha) = frame_motion(&smooth, time.elapsed()) else {
        // The tick frame itself is presented.
        shown.set_if_neq(MotionShown(false));
        return Ok(());
    };
    if tick.pages_version != gpu.atlas.version() {
        // The atlas changed after the tick frame was packed (it may have
        // started over): its frames may be gone, so the tick frame itself
        // is presented until the next one.
        shown.set_if_neq(MotionShown(false));
        return Ok(());
    }
    let shifts: Vec<(UnitKey, IVec2)> = tick
        .movers
        .values()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|k| (k, motion.shift(k, alpha)))
        .filter(|(_, d)| *d != IVec2::ZERO)
        .collect();
    let wanted = (tick.seq, shifts);
    if composed.as_ref() == Some(&wanted) {
        shown.set_if_neq(MotionShown(!wanted.1.is_empty()));
        return Ok(());
    }
    let shift_of = |k: UnitKey| {
        wanted
            .1
            .iter()
            .find(|(u, _)| *u == k)
            .map_or(IVec2::ZERO, |(_, d)| *d)
    };
    let screen = |i: usize| tick.packed.items[i].layer != LAYER_WORLD;
    let items = shifted_items(&tick.items, &tick.movers, shift_of, screen)
        .unwrap_or_else(|| tick.items.to_vec());
    let assets = &state.assets;
    let bins = crate::scene::bin(&items, &assets.frames, &assets.maps, tick.packed.view)?;
    let mut packed = tick
        .packed
        .with_items(&items, &bins, &assets.frames, gpu.atlas.slots())?;
    packed.set_layers(|i| tick.packed.items[i].layer);
    *seq += 1;
    commands.insert_resource(MotionJob {
        seq: *seq,
        tick_seq: tick.seq,
        packed: Arc::new(packed),
        pages_version: tick.pages_version,
        layers: layers.targets(),
    });
    shown.set_if_neq(MotionShown(!wanted.1.is_empty()));
    *composed = Some(wanted);
    Ok(())
}

/// The tick frame's `items` with every item of a moving unit moved by its
/// shift: the unit is found by the item's draw slot in `movers`, and
/// `shift` gives the unit's move. Items on the screen layer (`fixed`, by
/// index) stay. `None` when nothing moves, so the tick frame's own layers
/// stand.
pub fn shifted_items(
    items: &[DrawItem],
    movers: &BTreeMap<u64, UnitKey>,
    shift: impl Fn(UnitKey) -> IVec2,
    fixed: impl Fn(usize) -> bool,
) -> Option<Vec<DrawItem>> {
    let mut by_unit: BTreeMap<UnitKey, IVec2> = BTreeMap::new();
    let mut out: Option<Vec<DrawItem>> = None;
    for (i, item) in items.iter().enumerate() {
        let Some(&key) = movers.get(&item.key.slot()) else {
            continue;
        };
        let d = *by_unit.entry(key).or_insert_with(|| shift(key));
        if d == IVec2::ZERO || fixed(i) {
            continue;
        }
        let list = out.get_or_insert_with(|| items.to_vec());
        list[i].x += d.x;
        list[i].y += d.y;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{DrawKey, FrameId};

    const A: UnitKey = UnitKey::new(1, 7);
    const B: UnitKey = UnitKey::new(3, 7);

    /// A 16.16 position whose client pixels are `(x, y + 2048)`, on whole
    /// 1/2048 sub-tile steps (the client pixel grain).
    fn fixed(x: i32, y: i32) -> (u32, u32) {
        let a = x + 2 * y + 4096;
        let b = 2 * y - x + 4096;
        ((a as u32) << 11, (b as u32) << 11)
    }

    fn seen(key: UnitKey, x: i32, y: i32) -> Seen {
        Seen {
            key,
            pos: fixed(x, y),
            placements: 0,
            on_map: true,
        }
    }

    #[test]
    fn the_fixed_test_positions_land_on_their_pixels() {
        assert_eq!(client(fixed(100, 50)), ClientPos { x: 100, y: 2098 });
        assert_eq!(client(fixed(-7, 3)), ClientPos { x: -7, y: 2051 });
    }

    #[test]
    fn the_blend_is_taken_in_fixed_point() {
        let (p, c) = ((0, 1 << 16), (1 << 16, 0));
        assert_eq!(blend(p, c, 0.0), p);
        assert_eq!(blend(p, c, 1.0), c);
        assert_eq!(blend(p, c, 0.25), (1 << 14, 3 << 14));
        // Out-of-range fractions clamp.
        assert_eq!(blend(p, c, -1.0), p);
        assert_eq!(blend(p, c, 3.0), c);
    }

    #[test]
    fn a_walking_unit_glides_from_its_previous_tick_position() {
        let mut m = UnitMotion::default();
        m.observe(10, [seen(A, 100, 50)]);
        // Just appeared: nothing to blend from.
        assert_eq!(m.shift(A, 0.0), IVec2::ZERO);
        assert!(!m.moving());
        m.observe(11, [seen(A, 106, 53)]);
        assert!(m.moving());
        // Right after the tick it still stands where tick 10 put it.
        assert_eq!(m.shift(A, 0.0), IVec2::new(-6, -3));
        // Halfway: half the step left (rounded to whole pixels).
        assert_eq!(m.shift(A, 0.5), IVec2::new(-3, -2));
        assert_eq!(m.shift(A, 2.0 / 3.0), IVec2::new(-2, -1));
        // At the next tick: the tick frame's own position.
        assert_eq!(m.shift(A, 1.0), IVec2::ZERO);
        // Untracked units never move.
        assert_eq!(m.shift(B, 0.0), IVec2::ZERO);
    }

    #[test]
    fn teleports_spawns_map_changes_jumps_and_missed_ticks_snap() {
        let mut m = UnitMotion::default();
        m.observe(1, [seen(A, 0, 0)]);
        // A placement (teleport) snaps even for a short move.
        let mut t = seen(A, 4, 0);
        t.placements = 1;
        m.observe(2, [t]);
        assert_eq!(m.shift(A, 0.0), IVec2::ZERO);
        // The next walking step blends again.
        let mut w = seen(A, 8, 0);
        w.placements = 1;
        m.observe(3, [w]);
        assert_eq!(m.shift(A, 0.0), IVec2::new(-4, 0));
        // A step longer than the snap limit is a jump.
        let mut j = seen(A, 8 + SNAP_PX + 1, 0);
        j.placements = 1;
        m.observe(4, [j]);
        assert_eq!(m.shift(A, 0.0), IVec2::ZERO);
        // Leaving the map snaps.
        let mut off = seen(A, 8 + SNAP_PX + 3, 0);
        off.placements = 1;
        off.on_map = false;
        m.observe(5, [off]);
        assert_eq!(m.shift(A, 0.0), IVec2::ZERO);
        // Two ticks at once still blend; three do not.
        m.observe(6, [seen(B, 0, 0)]);
        m.observe(8, [seen(B, 4, 0)]);
        assert_eq!(m.shift(B, 0.0), IVec2::new(-4, 0));
        m.observe(11, [seen(B, 8, 0)]);
        assert_eq!(m.shift(B, 0.0), IVec2::ZERO);
        // A unit gone for a tick comes back without a step.
        m.observe(12, []);
        m.observe(13, [seen(B, 12, 0)]);
        assert_eq!(m.shift(B, 0.0), IVec2::ZERO);
    }

    #[test]
    fn the_same_tick_seen_again_keeps_the_step() {
        let mut m = UnitMotion::default();
        m.observe(1, [seen(A, 0, 0)]);
        m.observe(2, [seen(A, 4, 2)]);
        m.observe(2, [seen(A, 9, 9)]);
        assert_eq!(m.shift(A, 0.0), IVec2::new(-4, -2));
    }

    fn item(pass: u32, minor: u32, sub: u8, x: i32) -> DrawItem {
        let mut d = DrawItem::new(FrameId(0), x, 10);
        d.key = DrawKey::new(pass, 0, minor, sub).unwrap();
        d
    }

    #[test]
    fn a_moving_units_items_move_by_its_slot_and_others_stay() {
        // Unit A: body cels in slot (5, 0, 1) and its shadow in (3, 0, 1);
        // unit B (same GUID, other type) in slot (5, 0, 2); a tile.
        let items = vec![
            item(3, 1, 0, 100),
            item(5, 1, 0, 100),
            item(5, 1, 1, 104),
            item(5, 2, 0, 200),
            item(1, 9, 0, 0),
        ];
        let mut movers = BTreeMap::new();
        movers.insert(items[0].key.slot(), A);
        movers.insert(items[1].key.slot(), A);
        movers.insert(items[3].key.slot(), B);
        let shift = |k: UnitKey| {
            if k == A {
                IVec2::new(-3, 1)
            } else {
                IVec2::ZERO
            }
        };
        let out = shifted_items(&items, &movers, shift, |_| false).unwrap();
        assert_eq!(out[0].x, 97);
        assert_eq!((out[1].x, out[1].y), (97, 11));
        assert_eq!(out[2].x, 101);
        assert_eq!(out[3], items[3]);
        assert_eq!(out[4], items[4]);
        // Keys (and so the draw order) never change.
        assert!(out.iter().zip(&items).all(|(a, b)| a.key == b.key));
        // Items on the screen layer stay where the tick put them.
        let out = shifted_items(&items, &movers, shift, |i| i == 0).unwrap();
        assert_eq!(out[0], items[0]);
        assert_eq!(out[1].x, 97);
    }

    #[test]
    fn nothing_moving_keeps_the_tick_frame() {
        let items = vec![item(5, 1, 0, 100)];
        let mut movers = BTreeMap::new();
        movers.insert(items[0].key.slot(), A);
        assert!(shifted_items(&items, &movers, |_| IVec2::ZERO, |_| false).is_none());
        assert!(shifted_items(&items, &BTreeMap::new(), |_| IVec2::ONE, |_| false).is_none());
    }
}
