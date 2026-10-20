//! Helpers the Act II–V bodies share: the scheduling and mode helpers of
//! the `ai-bodies-*` summaries, the room scan of mode 0 with the unit
//! tests the scan callbacks use, the pack scan and the Vulture land /
//! take-off helpers (also FrogDemon's sink and surface).

use d2_data::tables::Skills;

use crate::game::Game;
use crate::units::{RoomId, UnitId, UnitType};

use super::tactics::*;
use super::{idle_keep_mode, mode, request_mode, AiHost, Ctx, ModeTarget, TickParam};

/// "wait N": delete the thinks and schedule one at frame +
/// N; the anim mode is not changed.
pub(super) fn wait<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId, n: i32) {
    idle_keep_mode(game, cx, u, n);
}

/// "mode m at (x, y)": path step count 1, mode m
/// at the point, no target unit.
pub(super) fn mode_point<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    m: u8,
    x: i32,
    y: i32,
) -> bool {
    cx.world.set_path_steps(u, 1);
    request_mode(game, cx, u, m, ModeTarget::Point(x, y))
}

/// The mode request `(x, y, m)`: mode m at the point
/// with no path step set.
pub(super) fn mode_point_raw<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    m: u8,
    x: i32,
    y: i32,
) -> bool {
    request_mode(game, cx, u, m, ModeTarget::Point(x, y))
}

/// "Skill k at U".
pub(super) fn skill_k<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    p: &TickParam,
    k: usize,
    target: Option<UnitId>,
) {
    let (s, m) = cx.skill(p, k);
    use_skill(game, cx, u, m, s, target_of(target));
}

/// A mode request's target: the unit, or the point (0, 0) for none.
pub(super) fn target_of(t: Option<UnitId>) -> ModeTarget {
    match t {
        Some(t) => ModeTarget::Unit(t),
        None => ModeTarget::Point(0, 0),
    }
}

/// (Δx)² + (Δy)² of the two positions (wrapping i32).
pub(super) fn sq_dist<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, a: UnitId, b: UnitId) -> i32 {
    let (ax, ay) = cx.world.position(a);
    let (bx, by) = cx.world.position(b);
    let (dx, dy) = (ax.wrapping_sub(bx), ay.wrapping_sub(by));
    dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy))
}

/// The unit's room.
pub(super) fn room_of(game: &Game, u: UnitId) -> Option<RoomId> {
    game.lists.unit(u).and_then(|e| e.room())
}

/// Unit type.
pub(super) fn type_of(game: &Game, u: UnitId) -> Option<UnitType> {
    game.lists.unit(u).map(|e| e.ty)
}

pub(super) fn is_monster(game: &Game, u: UnitId) -> bool {
    type_of(game, u) == Some(UnitType::Monster)
}

pub(super) fn is_player(game: &Game, u: UnitId) -> bool {
    type_of(game, u) == Some(UnitType::Player)
}

/// GUID (unit +0x0C); −1 for no unit.
pub(super) fn guid_of(game: &Game, u: Option<UnitId>) -> i32 {
    u.and_then(|u| game.lists.unit(u))
        .map_or(-1, |e| e.guid as i32)
}

/// Scan mode 0: every unit of every room of
/// the unit's room's adjacent list (own room included), in room order
/// and room unit-list order. The callbacks run over this list; a scan
/// that "returns the first" unit stops at it.
pub(super) fn scan_units(game: &Game, u: UnitId) -> Vec<UnitId> {
    let Some(room) = room_of(game, u) else {
        return Vec::new();
    };
    let rooms = game
        .lists
        .room(room)
        .map(|r| r.adjacent.clone())
        .unwrap_or_default();
    rooms
        .into_iter()
        .flat_map(|r| game.lists.room_units(r))
        .collect()
}

/// Monstats `BaseId` of a class; −1 for a missing row.
pub(super) fn base_id<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, class: i32) -> i32 {
    cx.monstats(class)
        .map_or(-1, |r| i32::from(r.baseid as i16))
}

/// `BaseId` of a unit's class.
pub(super) fn unit_base<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, u: UnitId) -> i32 {
    base_id(cx, cx.world.class(u))
}

/// A fixed class row: `class`, or −1 when monstats has ≤ `class` rows.
pub(super) fn fixed_class<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, class: i32) -> i32 {
    if (class as usize) < cx.tables.monstats.len() {
        class
    } else {
        -1
    }
}

/// M evil and u evil, or m good and u good (m
/// neutral never).
pub(super) fn pairing<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, m: UnitId, u: UnitId) -> bool {
    match cx.world.alignment(m) {
        0 => cx.world.alignment(u) == 0,
        2 => cx.world.alignment(u) == 2,
        _ => false,
    }
}

/// Anim mode 0 or 12 (death, dead).
pub(super) fn dying<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, u: UnitId) -> bool {
    matches!(cx.world.anim_mode(u), mode::DEATH | mode::DEAD)
}

/// Life (stat 6) of a unit.
pub(super) fn life<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, u: UnitId) -> i32 {
    cx.world.stat(u, 6)
}

/// The unit's class lists the skill among
/// `Skill1`..`Skill8`.
pub(super) fn has_skill<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, u: UnitId, skill: i32) -> bool {
    let class = cx.world.class(u);
    cx.monstats(class).is_some() && (1..=8).any(|k| cx.class_skill(class, k).0 == skill)
}

/// The skills row of a skill id.
pub(super) fn skill_row<'a, W: AiHost + ?Sized>(cx: &Ctx<'a, W>, skill: i32) -> Option<&'a Skills> {
    usize::try_from(skill)
        .ok()
        .and_then(|s| cx.tables.skills.get(s))
}

/// `aurastate` / `auratargetstate` of a skills row as signed values.
pub(super) fn aura_states(row: &Skills) -> (i32, i32) {
    (
        i32::from(row.aurastate as i16),
        i32::from(row.auratargetstate as i16),
    )
}

/// "teleport in range r": x := own x +
/// roll(2r) − r, then y likewise (two steps, x first; no draw when 2r <
/// 1); then the skill use `(mode, skill, 0, x, y)`.
pub(super) fn teleport_in_range<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    r: i32,
    skill: i32,
    m: u8,
) {
    let n = r.wrapping_mul(2);
    let (ox, oy) = cx.world.position(u);
    let dx = cx.world.seed(u).roll(n) as i32;
    let x = ox.wrapping_add(dx).wrapping_sub(r);
    let dy = cx.world.seed(u).roll(n) as i32;
    let y = oy.wrapping_add(dy).wrapping_sub(r);
    use_skill(game, cx, u, m, skill, ModeTarget::Point(x, y));
}

/// Per axis |Δ| − (size(a) /
/// 2 + size(b) / 2), clamped at 0; then (2·max + min) / 2.
pub(super) fn reach_distance<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, a: UnitId, b: UnitId) -> i32 {
    let ((ax, ay), (bx, by)) = (cx.world.position(a), cx.world.position(b));
    let gap = cx.world.size(a) / 2 + cx.world.size(b) / 2;
    let dx = bx.wrapping_sub(ax).wrapping_abs().wrapping_sub(gap).max(0);
    let dy = by.wrapping_sub(ay).wrapping_abs().wrapping_sub(gap).max(0);
    distance_formula(dx, dy)
}

/// Max(|dx|, |dy|) +
/// min(|dx|, |dy|) / 2 from the unit's position.
pub(super) fn point_distance<W: AiHost + ?Sized>(
    cx: &Ctx<'_, W>,
    u: UnitId,
    x: i32,
    y: i32,
) -> i32 {
    let (ux, uy) = cx.world.position(u);
    let (dx, dy) = (
        ux.wrapping_sub(x).wrapping_abs(),
        uy.wrapping_sub(y).wrapping_abs(),
    );
    dx.max(dy) + dx.min(dy) / 2
}

/// "wander near T n" in a target-mode-1 body. T = 0 is
/// unreachable there (no mode-1 or mode-4 body runs without a
/// target): asserted, not handled.
pub(super) fn wander_near_opt<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    t: Option<UnitId>,
    n: i32,
) -> bool {
    let t = t.expect("wander near target 0 in a target-mode-1 body (ai.md §2.3)");
    wander_near(game, cx, u, t, n)
}

/// "walk in radius of t (a, b)". reads t
/// without a null test (an access violation in the base game), so t = 0 is
/// unreachable (no mode-1 or mode-4 body runs without a target):
/// asserted, not handled.
pub(super) fn radius<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    t: Option<UnitId>,
    a: i32,
    b: i32,
) -> bool {
    let t = t.expect("walk in radius of target 0 (ai.md §2.3)");
    walk_in_radius(game, cx, u, t, a, b)
}

/// The pack scan (scan 1, callback):
/// the nearest other monster with the scanner's `BaseId` not in mode 0
/// or 12, by squared distance (strictly smaller wins); with that squared
/// distance.
pub(super) fn pack_scan<W: AiHost + ?Sized>(
    game: &Game,
    cx: &Ctx<'_, W>,
    u: UnitId,
) -> Option<(UnitId, i32)> {
    let base = unit_base(cx, u);
    let mut best: Option<(UnitId, i32)> = None;
    for v in scan_units(game, u) {
        if v == u || !is_monster(game, v) || unit_base(cx, v) != base || dying(cx, v) {
            continue;
        }
        let d = sq_dist(cx, u, v);
        if d < best.map_or(0x7FFF_FFFF, |b| b.1) {
            best = Some((v, d));
        }
    }
    best
}

/// Unit flags the Vulture land / take-off helpers toggle.
pub(super) const FLAG_LANDED: u32 = 0x0E;

/// Land: unit flags |= 0x0E first;
/// pattern already 1 → false; place at the own position (failure →
/// false); stamp pattern 1 with mask 0x100; pattern := 1; move mask :=
/// 0x3C01; wait 12; true.
pub(super) fn land<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) -> bool {
    cx.world.set_unit_flag(u, FLAG_LANDED);
    if cx.world.path_pattern(u) == 1 {
        return false;
    }
    let room = room_of(game, u);
    let (x, y) = cx.world.position(u);
    if !cx.world.place_unit(game, u, room, x, y) {
        return false;
    }
    cx.world.stamp_pattern(game, room, x, y, 1, 0x100);
    cx.world.set_path_pattern(u, 1);
    cx.world.set_move_mask(u, 0x3C01);
    wait(game, cx, u, 12);
    true
}

/// Take off: unit flags &= ~0x0E; clear bit 0x1000 then bit
/// 0x100 of the one cell at the own position (own room); pattern := 5;
/// move mask := 0; wait 12.
pub(super) fn take_off<W: AiHost + ?Sized>(game: &mut Game, cx: &mut Ctx<'_, W>, u: UnitId) {
    cx.world.clear_unit_flag(u, FLAG_LANDED);
    let room = room_of(game, u);
    let (x, y) = cx.world.position(u);
    cx.world.clear_cell(game, room, x, y, 0x1000);
    cx.world.clear_cell(game, room, x, y, 0x100);
    cx.world.set_path_pattern(u, 5);
    cx.world.set_move_mask(u, 0);
    wait(game, cx, u, 12);
}

/// What the spawn info gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SpawnInfo {
    pub class: i32,
    pub x: i32,
    pub y: i32,
    pub mode: u8,
}

/// "Chain(b)": the class chain walk `(clamp(b), p)`, the
/// `BaseId` of row b walked p steps along `NextInClass`,
/// p := the chain position of
/// the unit's class (monstats +0x4B; class −1 → 0).
pub(super) fn chain<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, u: UnitId, b: i32) -> i32 {
    walk_chain(cx, base_id(cx, fixed_class(cx, b)), {
        let class = cx.world.class(u);
        if class < 0 {
            0
        } else {
            cx.world.chain_index(class)
        }
    })
}

/// `y` walked `n` steps along `NextInClass`.
fn walk_chain<W: AiHost + ?Sized>(cx: &Ctx<'_, W>, mut y: i32, n: i32) -> i32 {
    for _ in 0..n.max(0) {
        y = cx
            .monstats(y)
            .map_or(-1, |r| i32::from(r.nextinclass as i16));
    }
    y
}

/// baalclone, the incoming class key 544 tests.
const BAALCLONE: i32 = 570;
/// State 146 set by the ancient statue key.
const STATE_146: u16 = 146;

/// The spawn info with (unit, &class, &x, &y, &mode, difficulty,
/// pick 0), keyed by the unit's `BaseId`.
/// `class_in` is the incoming class (read by key 544 only).
pub(super) fn spawn_info<W: AiHost + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    u: UnitId,
    class_in: i32,
) -> SpawnInfo {
    let class = cx.world.class(u);
    let key = if is_monster(game, u) && cx.monstats(class).is_some() {
        let b = base_id(cx, class);
        if cx.monstats(b).is_some() {
            b
        } else {
            -1
        }
    } else {
        -1
    };
    let (ux, uy) = cx.world.position(u);
    let (tx, ty) = cx.world.path_target_point(u);
    let at = |class, x, y, mode| SpawnInfo { class, x, y, mode };
    match key {
        206 => at(chain(cx, u, 15), ux, uy + 3, 1),
        228 => {
            let c = fixed_class(cx, 96);
            let c = cx.world.class_for_level(game, room_of(game, u), c);
            at(c, ux, uy + 2, 1)
        }
        267 => at(fixed_class(cx, 6), tx, ty, 8),
        284 => at(chain(cx, u, 68), ux + 8, uy, 8),
        298 => at(chain(cx, u, 301), tx, ty, 1),
        321 => {
            let c = if class == 711 {
                fixed_class(cx, 712)
            } else {
                fixed_class(cx, 19)
            };
            at(c, ux, uy, 1)
        }
        334 => at(chain(cx, u, 114), ux - 2, uy - 2, 1),
        484 => at(chain(cx, u, 453), ux, uy + 3, 1),
        526 | 528 => panic!("spawn info key {key} without a pick (ai-bodies-2.md §13.1, fatal)"),
        537 => {
            let c = chain(cx, u, 540);
            cx.world.set_state(game, u, STATE_146, true);
            at(c, ux, uy + 2, 1)
        }
        544 => {
            // The incoming class compared with 570.
            let (c, m) = if class_in == BAALCLONE {
                (class_in, 1)
            } else {
                let r = cx.world.seed(u).roll(2) as i32 + i32::from(cx.info.difficulty);
                let c = walk_chain(cx, base_id(cx, fixed_class(cx, 562)), r);
                (c, 4)
            };
            let x = ux + cx.world.seed(u).roll(24) as i32 - 12;
            let y = uy + cx.world.seed(u).roll(24) as i32 - 12;
            at(c, x, y, m)
        }
        _ => at(fixed_class(cx, 0), 0, 0, 1),
    }
}
