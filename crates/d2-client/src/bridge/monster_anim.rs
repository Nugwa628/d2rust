//! The client monster's animation: the frame (+0x44, 8.8), its count
//! (+0x48) and speed (+0x4C). A mode set restarts the
//! frame at 0 with the mode's AnimData count and the rate
//! (through `d2_sim::units::anim_rate`); the add's
//! set-up draws the first frame from the unit seed; the monster update
//! advances it by the mode
//! record's anim kind,
//! a mode-14 cast through its `monseq` sequence, and ends
//! the mode.

use d2_sim::rng::Seed;
use d2_sim::units::anim::advance_frame;
use d2_sim::units::anim_rate::{anim_rate, mode_row, Rate, RateInput};
use d2_sim::units::record::{Anim, AnimRecord};

pub use d2_sim::skills::sequences::{MonsterSequences, SeqFrame};

use super::world::{
    ClientUnit, ClientWorld, ModelInputs, MonsterClass, MonsterSequence, UnitKey, MONSTER,
};

/// Stats of the rate.
const STAT_VELOCITYPERCENT: u16 = 67;
const STAT_ATTACKRATE: u16 = 68;
const STAT_OTHER_ANIMRATE: u16 = 69;

/// The speed (+0x4C) of a monster in its current mode:
/// draw type 1, the class's AnimData speed for the mode, the unit's
/// totals of stats 67–69, w = the class's run speed in mode 15 (RN), else
/// its walk speed, and the path velocity-mode test
/// (`npc` monsters: modes 2 and 15; others: the mode row's V column).
/// No item values, used skill or dual wield on the client.
/// PROVISIONAL: the client reads no used
/// skill, so the skill columns (V-skill, A-skill) never apply; settled by
/// a capture of a monster casting in sight.
pub fn rate(w: &ClientWorld, inputs: &ModelInputs, key: UnitKey) -> i32 {
    let Some(u) = w.units.get(&key) else {
        return 0;
    };
    rate_of(u, inputs, |s| w.total(key, s, 0))
}

/// [`rate`] of a unit `u` whose stat totals `total` gives (set C has no
/// state or item lists: its base stats). Without a class row 0; when the
/// rate sets no speed, the unit's own speed.
pub fn rate_of(u: &ClientUnit, inputs: &ModelInputs, total: impl Fn(u16) -> i32) -> i32 {
    let Some(class) = inputs
        .tables
        .monsters
        .get(u.class as usize)
        .and_then(|c| c.as_ref())
    else {
        return 0;
    };
    let m = u.mode;
    let s = class
        .anims
        .get(m as usize)
        .copied()
        .flatten()
        .map_or(0, |(_, s)| s as i32);
    let velocity_mode = if class.npc {
        matches!(m, 2 | 15)
    } else {
        mode_row(1, u.class, m).v
    };
    let i = RateInput {
        applies: true,
        t: 1,
        c: u.class,
        m,
        s,
        velocitypercent: total(STAT_VELOCITYPERCENT),
        attackrate: total(STAT_ATTACKRATE),
        other_animrate: total(STAT_OTHER_ANIMRATE),
        has_path: true,
        w: i32::from(if m == 15 {
            class.run_speed
        } else {
            class.walk_speed
        }),
        velocity_mode,
        ..RateInput::default()
    };
    match anim_rate(&i) {
        Ok(Rate::Set { speed, .. }) => speed,
        _ => u.speed.unwrap_or(0),
    }
}

/// The mode set's animation part: frame
/// := 0, count := the mode's AnimData frames << 8, speed := [`rate`].
///
/// A mode set to the unit's current mode only queues the unit and sets
/// flag 0x1: the animation is not restarted
/// ([`mode_init`] is the creation call, which always sets it up).
pub fn mode_set(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, mode: u32) {
    // clears flag 0x40 before the set, the same mode too.
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    u.flag_40 = false;
    if u.mode == mode {
        return;
    }
    mode_init(w, inputs, key, mode);
    // A new mode: the hit overlays of the mode set.
    super::modes::hit_overlays(w, inputs, key, mode);
}

/// The mode set of a monster request into the sequence mode 14
/// (codes 0x15 / 0x16): before the mode set, the
/// sequence preload walks the sequence's frames and writes
/// each frame's own mode to the unit's mode word (+0x10) while it loads
/// that mode's graphics, so the mode set always sees a changed mode and
/// reloads the sequence, the hit overlays
/// included, even when the unit is already in mode 14 (MagottUp after
/// MagottDown restarts at frame 0 with count 3072).
pub fn mode_set_sequence(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, mode: u32) {
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    u.flag_40 = false;
    mode_init(w, inputs, key, mode);
    super::modes::hit_overlays(w, inputs, key, mode);
}

/// The animation part of the mode set unconditionally (unit creation:
/// the allocation stored the mode already).
pub fn mode_init(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, mode: u32) {
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    if u.mode != mode {
        u.mode_changes = u.mode_changes.wrapping_add(1);
    }
    u.mode = mode;
    let row = inputs
        .tables
        .monsters
        .get(u.class as usize)
        .and_then(|c| c.as_ref());
    let frames = row
        .and_then(|c| c.anims.get(mode as usize).copied().flatten())
        .map_or(0, |(f, _)| f as i32);
    u.anim_events = row.and_then(|c| c.anim_events.get(mode as usize).copied().flatten());
    u.action_frame = 0;
    u.frame = 0;
    u.frame_count = frames << 8;
    u.sequence = None;
    u.anim_restarts = u.anim_restarts.wrapping_add(1);
    // Mode 14 with a sequence for the used skill: the sequence load
    // replaces the AnimData count.
    if mode == 14 {
        if let Some(frames) = u
            .used_skill
            .and_then(|s| inputs.tables.monster_sequences.lookup(u.class as usize, s))
        {
            sequence_load(u, frames);
        }
    }
    // The rate: a client monster has no used-skill entry,
    // so in mode 14 it falls to the speed alone and the
    // sequence step stays 256.
    let speed = rate(w, inputs, key);
    if let Some(u) = w.units.get_mut(&key) {
        u.speed = Some(speed);
    }
}

/// Frame := roll(+0x48) on the unit seed
/// (range rule). A unit with no seed in the
/// model keeps frame 0.
pub fn first_frame(w: &mut ClientWorld, key: UnitKey) {
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    let Some((lo, hi)) = u.seed else {
        return;
    };
    let mut s = Seed::new(lo, hi);
    u.frame = s.roll(u.frame_count) as i32;
    u.seed = Some((s.lo, s.hi));
}

/// The initial path direction of a monster of
/// set S that is not an `npc`: 0, or the low 6 bits of one step of the
/// unit seed when the class has mode 2 (`modes` bit 2);
/// then by `BaseId`: 96 (`mummy1` family) → 7, 301
/// (`vilechild1` family) → [`vile_mother_dir`]; then classes 351 → 0x18,
/// 353 → 0x28, 352, 357, 344 → 0. Measured on the base game: mummy 96
/// `d` 7, vile child 301 `d` 0 (no mother beside it).
pub fn first_direction(
    w: &mut ClientWorld,
    monsters: &[Option<MonsterClass>],
    key: UnitKey,
    row: &MonsterClass,
) {
    if row.npc {
        return;
    }
    let mother = if row.base_id == 301 {
        vile_mother_dir(w, monsters, key)
    } else {
        0
    };
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    let mut dir = 0;
    if row.modes & (1 << 2) != 0 {
        if let Some((lo, hi)) = u.seed {
            let mut s = Seed::new(lo, hi);
            dir = (s.step() & 0x3F) as u8;
            u.seed = Some((s.lo, s.hi));
        }
    }
    match row.base_id {
        96 => dir = 7,
        301 => dir = mother,
        _ => {}
    }
    dir = match u.class {
        351 => 0x18,
        353 => 0x28,
        352 | 357 | 344 => 0,
        _ => dir,
    };
    u.path_dir = Some(dir);
}

/// The mother search for a new vile child U: the first monster
/// V of the rooms near U for which the filter returns nonzero
/// (that value), else 0. The filter: V's class `BaseId` is 298
/// (`vilemother1`), the reach distance with (U, V) ≤ 1 (per axis
/// |Δ| − (size(U) / 2 + size(V) / 2), clamped at 0, then (2·max + min) / 2;
/// size `monstats2` `SizeX`), and U stands at V + 3·(dx, dy) of offset pair
/// e = [6, 7, 0, 1, 2, 3, 4, 5][dir8(V's direction)]:
/// the child born in front of its mother takes her
/// direction (a direction of 0 reads as no match, the scan goes on). The
/// model scans every monster it holds (the near-room list is a superset
/// test: a match is adjacent to U).
fn vile_mother_dir(w: &ClientWorld, monsters: &[Option<MonsterClass>], key: UnitKey) -> u8 {
    use d2_sim::skills::use_::bodies::b4_helpers::{dir8, pair};
    const E: [usize; 8] = [6, 7, 0, 1, 2, 3, 4, 5];
    let row = |u: &ClientUnit| monsters.get(u.class as usize).and_then(|c| c.as_ref());
    let Some(child) = w.units.get(&key) else {
        return 0;
    };
    let Some(cp) = child.position else {
        return 0;
    };
    let size = |u: &ClientUnit| row(u).map_or(0, |c| i32::from(c.size_x));
    for v in w.units.values() {
        if v.key == key || v.key.unit_type != MONSTER {
            continue;
        }
        let (Some(vp), Some(vrow), Some(d)) = (v.position, row(v), v.path_dir) else {
            continue;
        };
        if vrow.base_id != 298 {
            continue;
        }
        let (ux, uy) = (i32::from(cp.0), i32::from(cp.1));
        let (vx, vy) = (i32::from(vp.0), i32::from(vp.1));
        let gap = size(child) / 2 + size(v) / 2;
        let ax = ((vx - ux).abs() - gap).max(0);
        let ay = ((vy - uy).abs() - gap).max(0);
        if (2 * ax.max(ay) + ax.min(ay)) / 2 > 1 {
            continue;
        }
        let (dx, dy) = pair(E[dir8(i32::from(d)) as usize]);
        if vx + 3 * dx == ux && vy + 3 * dy == uy && d != 0 {
            return d;
        }
    }
    0
}

/// Whether the draw that follows the update writes unit `u` back: both
/// write-back sites (in the composite)
/// come after the COF box pre-test
/// (`x_min + X < W − 1`, `x_max + X ≥ 0`,
/// `y_max + Y ≥ 0`, `y_min + Y < H − 1`, (X, Y) the unit's screen
/// position: X = client x − the unit origin x of
/// [`super::world::UnitOrigin`], Y likewise), and the draw skips a unit
/// with flag-ex bit 18. PROVISIONAL:
/// the bridge holds no COF box, so the box is taken as the
/// point (0, 0, 0, 0): a corpse whose box reaches into the view while its
/// point is outside is not written back. Measured on the base game: a
/// Fallen killed 40 sub-tiles east of the player keeps the server's `d`
/// 20. No frame drawn yet: drawn.
fn drawn(w: &ClientWorld, u: &ClientUnit) -> bool {
    use crate::rules::camera::moving_to_client;
    use crate::rules::draw_order::UNIT_EX_SKIP;
    if u.flag_ex & UNIT_EX_SKIP != 0 {
        return false;
    }
    let Some(o) = w.unit_origin else {
        return true;
    };
    let Some((x, y)) = u.position else {
        return true;
    };
    let (px, py) = u
        .precise
        .unwrap_or(((u32::from(x) << 16) | 0x8000, (u32::from(y) << 16) | 0x8000));
    let at = moving_to_client(px, py);
    let (sx, sy) = (at.x - o.x, at.y - o.y);
    let h = o.play_height + 40;
    sx < o.width - 1 && sx >= 0 && sy >= 0 && sy < h - 1
}

/// The draw's snap write-back on a dead monster: drawn in mode DT (0)
/// or DD (12), a monster whose
/// `monstats2` `d<mode>` is 8 gets the expected count 4 while its
/// class's graphics-ready flag for the mode is 0, else `d<mode>`; when
/// that count `n` differs from the COF's direction count `D`
/// ([`MonsterClass::cof_dirs`]), its path direction becomes the snap of
/// (`D`, `n`): (8, 4) `table[dir64 >> 3] << 3`, table 1, 1, 3, 3, 5, 5,
/// 7, 7, or (16, 8); any other pair writes the direction back unchanged.
/// Each draw repeats it; the snapped value is a fixed point. A class
/// whose COF was not read (`D` = 0) takes `D` := `d<mode>`.
/// The flag is a host input (the preload scan that sets it
/// runs on the host's wall clock): the stored flag of the tick's draw
/// ([`ClientWorld::gfx_ready_now`]) where a check run has one, else 0 for
/// every class (row 19 reads 0 in every mode through a Fallen kill).
/// Only a drawn monster is written back ([`drawn`]): one killed off
/// screen keeps the server's direction (`d` 20 from its death on, no
/// write-back).
pub fn death_snap(w: &mut ClientWorld, inputs: &ModelInputs) {
    use super::critter_path::{self, Who};
    use crate::rules::unit_composite::{expected_directions, snap, DirectionSource};
    let rows = &inputs.tables.monsters;
    let class_dirs = |u: &ClientUnit| {
        let mode = usize::try_from(u.mode).ok().filter(|&m| m < 16)?;
        let row = rows.get(usize::try_from(u.class).ok()?)?.as_ref()?;
        let d = row.directions[mode];
        let cof = match row.cof_dirs[mode] {
            0 => d,
            c => c,
        };
        Some((d, cof, u.mode))
    };
    let mut dead: Vec<(Who, u8, u8, i32, u32)> = Vec::new();
    for u in w.units.values().filter(|u| u.key.unit_type == MONSTER) {
        if let Some((d, cof, mode)) = class_dirs(u).filter(|_| u.is_dead() && drawn(w, u)) {
            dead.push((Who::server(u.key), d, cof, mode as i32, u.class));
        }
    }
    for u in w
        .objclient
        .set_c
        .values()
        .filter(|u| u.key.unit_type == MONSTER)
    {
        if let Some((d, cof, mode)) = class_dirs(u).filter(|_| u.is_dead() && drawn(w, u)) {
            dead.push((Who::from(u.key), d, cof, mode as i32, u.class));
        }
    }
    for (who, d, cof, mode, class) in dead {
        let source = DirectionSource::Monster {
            d_mode: d,
            mode_table_zero: !w
                .gfx_ready_now()
                .is_some_and(|r| r.ready(class, mode as u32)),
        };
        let n = expected_directions(source, mode);
        if n == cof {
            continue;
        }
        let Some(dir) = critter_path::direction(w, who).or_else(|| {
            who.server
                .then(|| w.units.get(&who.key)?.path_dir)
                .flatten()
        }) else {
            continue;
        };
        let snapped = snap(cof, n, dir);
        if !critter_path::set_direction(w, who, snapped) && who.server {
            if let Some(u) = w.units.get_mut(&who.key) {
                u.path_dir = Some(snapped & 0x3F);
            }
        }
    }
}

/// The freeze gate at the head of the monster update:
/// a monster in state 1 `freeze`
/// that is not dead runs no step of
/// the update this tick (no anim advance, no end, no turn).
pub fn frozen(u: &ClientUnit) -> bool {
    u.states.contains(&STATE_FREEZE) && !u.is_dead()
}

/// `states.txt` row 1 `freeze`.
const STATE_FREEZE: u8 = 1;

/// The mode record {path kind, anim kind, end kind} of a
/// monster of `class` in `mode`; `None`: no class
/// row or mode ≥ 16 (the update does nothing).
pub fn mode_record(inputs: &ModelInputs, class: u32, mode: u32) -> Option<(u8, u8, u8)> {
    const NORMAL: [(u8, u8, u8); 16] = [
        (0, 2, 0),
        (0, 0, 0),
        (1, 0, 1),
        (0, 1, 2),
        (2, 1, 3),
        (2, 1, 3),
        (0, 1, 2),
        (0, 1, 2),
        (0, 1, 2),
        (0, 1, 2),
        (0, 2, 0),
        (0, 0, 0),
        (0, 2, 0),
        (1, 2, 2),
        (2, 1, 2),
        (1, 0, 1),
    ];
    const MOVE: [(u8, u8, u8); 16] = [
        (1, 2, 0),
        (0, 1, 2),
        (0, 1, 2),
        (0, 1, 2),
        (1, 0, 4),
        (1, 0, 4),
        (2, 1, 3),
        (1, 0, 4),
        (1, 0, 4),
        (1, 0, 4),
        (0, 2, 0),
        (1, 2, 0),
        (0, 1, 2),
        (0, 1, 2),
        (0, 1, 2),
        (0, 1, 2),
    ];
    let c = inputs
        .tables
        .monsters
        .get(class as usize)
        .and_then(|c| c.as_ref())?;
    let m = usize::try_from(mode).ok().filter(|&m| m < 16)?;
    Some(match (class, mode) {
        (243 | 333, 11) => (2, 1, 3),
        (543 | 544 | 570, 10) => (1, 0, 4),
        _ if c.move_modes & (1 << m) != 0 => MOVE[m],
        (_, 0) if c.base_id == 78 => MOVE[0],
        _ => NORMAL[m],
    })
}

/// The sequence load of a monster set to mode 14 (and of a
/// player set to mode 18, [`super::player_anim::mode_init`]):
/// +0x30 := the frames, +0x34 := their length
/// · 256, +0x38 := 0, +0x3C := 256, +0x48 := their count · 256; then the
/// frame setup of position 0.
/// Length = count = the number of `monseq` rows of the sequence.
pub fn sequence_load(u: &mut ClientUnit, frames: &[SeqFrame]) {
    let n = frames.len() as i32;
    let mut seq = MonsterSequence {
        frames: frames.to_vec(),
        length: n << 8,
        pos: 0,
        step: 0x100,
        drawn_mode: u.mode,
    };
    u.frame_count = n << 8;
    frame_setup(u, &mut seq, 0);
    u.sequence = Some(seq);
}

/// The frame setup with the old position `b`: +0x44 := the
/// frame of the frame at the position · 256, +0x40 := its mode, +0x4E :=
/// its event (position = b), else the last non-zero event of the frames
/// (b >> 8) + 1..= position >> 8 (0 when none or the range is empty).
fn frame_setup(u: &mut ClientUnit, seq: &mut MonsterSequence, b: i32) {
    let a = seq.pos;
    let at = |i: i32| usize::try_from(i).ok().and_then(|i| seq.frames.get(i));
    if let Some(f) = at(a >> 8) {
        u.frame = i32::from(f.frame) << 8;
        seq.drawn_mode = u32::from(f.mode);
    }
    let event = |i: i32| at(i).map_or(0, |f| f.event);
    u.action_frame = if a == b {
        event(a >> 8)
    } else {
        ((b >> 8) + 1..=a >> 8)
            .rev()
            .map(event)
            .find(|&e| e != 0)
            .unwrap_or(0)
    };
}

/// The frame advance. With a sequence: +0x4E := 0, position
/// += step (minus the length once when it reaches it), count −= step,
/// then the frame setup. Without: frame += speed (signed 16-bit), wrapping
/// at the count, noting the AnimData events crossed (no frame bonus).
pub fn advance(u: &mut ClientUnit) {
    advance_at(u, d2_sim::tick::TickRate::BASE, 0);
}

pub fn advance_at(u: &mut ClientUnit, rate: d2_sim::tick::TickRate, frame: i32) {
    if let Some(mut seq) = u.sequence.take() {
        u.action_frame = 0;
        let b = seq.pos;
        let step = rate.share(seq.step, frame);
        let mut p = b.wrapping_add(step);
        if p >= seq.length {
            p = p.wrapping_sub(seq.length);
        }
        seq.pos = p;
        u.frame_count = u.frame_count.wrapping_sub(step);
        frame_setup(u, &mut seq, b);
        u.sequence = Some(seq);
        return;
    }
    // The frame advance (no sequence, no frame bonus): the
    // frame and +0x4E, the event byte of the frames crossed.
    let mut a = Anim {
        sequence: None,
        frame: u.frame,
        frame_count: u.frame_count,
        speed: rate.share(i32::from(u.speed.unwrap_or(0) as i16), frame) as i16,
        action_frame: 0,
        record: u.anim_events.map(|events| AnimRecord {
            frames: (u.frame_count >> 8) as u32,
            byte_0f: 0,
            events,
        }),
    };
    advance_frame(&mut a, 0);
    u.frame = a.frame;
    u.action_frame = a.action_frame;
}

/// The mode missile gate on the client unit: the mode's `monstats` missile
/// when the latest advance noted an event (+0x4E ≠ 0) and the frames
/// ((cur − speed) >> 8, cur >> 8] hold an attack event (1 or 2), the scan
/// from (cur − speed) >> 8 finding none at that first frame;
/// else none.
fn mode_missile_of(
    inputs: &ModelInputs,
    u: &ClientUnit,
    rate: d2_sim::tick::TickRate,
    frame: i32,
) -> Option<u16> {
    if u.action_frame == 0 {
        return None;
    }
    let speed = rate.share(u.speed.unwrap_or(0), frame);
    let (hi, lo) = (u.frame >> 8, u.frame.wrapping_sub(speed) >> 8);
    if hi <= lo {
        return None;
    }
    let events = u.anim_events?;
    let event = |i: i32| {
        usize::try_from(i)
            .ok()
            .filter(|&i| i <= 0x8F)
            .and_then(|i| events.get(i))
            .copied()
            .unwrap_or(0)
    };
    let first = (lo..=hi).find(|&i| matches!(event(i), 1 | 2))?;
    if first <= lo {
        return None;
    }
    inputs
        .tables
        .monsters
        .get(u.class as usize)
        .and_then(|c| c.as_ref())
        .and_then(|c| c.mode_missiles.get(u.mode as usize).copied().flatten())
}

/// The skill step of the monster update,
/// before the path step: a
/// monster with a used skill S (a valid `skills` row) runs the generic do
/// (the client do of S at the request's level, w = 0) when
/// the previous advance's event byte (+0x4E) is 4, or is 1–3 with flag
/// 0x40 (+0xC4) clear. The do sets flag 0x40, so it runs
/// once per mode until the next mode set clears it.
/// First the entry-flags bit 0 path (a moving skill, the used entry E's
/// +0x0C bit 0): when the path step
/// ([`super::critter_path::server_path_step`]) reports the movement
/// ended, E.flags |= 2 unless the skill's `srvdofunc` is 67, the do runs,
/// and the event-byte do then runs only for byte 4. Otherwise, by the
/// skill's `srvdofunc` (the inferno skills and DiabLight): 95 with a
/// `monstats2` row → the do, then frame > `InfernoAnim` →
/// frame := `InfernoRollback` · 256; 126 → the do, then the frame's whole
/// part ≥ 0xC00 → frame := 0x800; 152 with a `monstats2` row and state 12
/// on U → frame ≥ `InfernoAnim` → the do; frame ≥ `InfernoRollback` →
/// frame := (one roll of 4 on U's seed + `InfernoRollback` − 4) · 256
/// (frames read after the bit-0 path, before the do).
pub fn skill_step(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Result<(), super::dispatch::HandlerError> {
    let Some(u) = w.units.get(&key) else {
        return Ok(());
    };
    if key.unit_type != MONSTER {
        return Ok(());
    }
    let Some(skill) = u.used_skill.and_then(|s| u16::try_from(s).ok()) else {
        return Ok(());
    };
    if inputs.tables.skills.get(usize::from(skill)).is_none() {
        return Ok(());
    }
    // The entry's level with bonuses: its base level; a monster has no
    // item skill bonuses.
    let level = u.used_skill_level;
    let srvdofunc = inputs.tables.skills[usize::from(skill)].srvdofunc;
    let flags = super::skills::used_entry(w, key).map_or(0, |e| e.flags);
    let mut free = true;
    if flags & 1 != 0 && super::critter_path::server_path_step(w, inputs, key) {
        if srvdofunc != 67 {
            if let Some(e) = super::skills::used_entry_mut(w, key) {
                e.flags |= 2;
            }
        }
        super::client_do::client_do(w, inputs, key, skill, level)?;
        free = false;
    }
    let Some(u) = w.units.get(&key) else {
        return Ok(());
    };
    let run = u.action_frame == 4 || (free && !u.flag_40 && matches!(u.action_frame, 1..=3));
    if run {
        super::client_do::client_do(w, inputs, key, skill, level)?;
        return Ok(());
    }
    let class = inputs
        .tables
        .monsters
        .get(u.class as usize)
        .and_then(|c| c.as_ref())
        .map(|c| (i32::from(c.inferno_anim), i32::from(c.inferno_rollback)));
    let frame = u.frame;
    let inferno = u.states.contains(&super::use_state::STATE_INFERNO);
    match srvdofunc {
        95 => {
            if let Some((anim, rollback)) = class {
                super::client_do::client_do(w, inputs, key, skill, level)?;
                if let Some(u) = w.units.get_mut(&key) {
                    if anim < u.frame >> 8 {
                        u.frame = rollback << 8;
                    }
                }
            }
        }
        126 => {
            super::client_do::client_do(w, inputs, key, skill, level)?;
            if let Some(u) = w.units.get_mut(&key) {
                if u.frame & !0xFF > 0xBFF {
                    u.frame = 0x800;
                }
            }
        }
        152 => {
            if let (Some((anim, rollback)), true) = (class, inferno) {
                if anim <= frame >> 8 {
                    super::client_do::client_do(w, inputs, key, skill, level)?;
                }
                if rollback <= frame >> 8 {
                    let r = monster_roll(w, key, 4);
                    if let Some(u) = w.units.get_mut(&key) {
                        u.frame = (r + rollback - 4) << 8;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// The seed roll of n on a monster: one step of its seed, the roll
/// of n.
fn monster_roll(w: &mut ClientWorld, key: UnitKey, n: i32) -> i32 {
    let Some(u) = w.units.get_mut(&key) else {
        return 0;
    };
    let Some((lo, hi)) = u.seed else {
        return 0;
    };
    let mut seed = Seed::new(lo, hi);
    let r = seed.roll(n) as i32;
    u.seed = Some((seed.lo, seed.hi));
    r
}

/// The mode missile step of the monster update,
/// after the anim step: a monster whose
/// mode missile m the gate gives creates the client missile m with record
/// flags 0x20 (0x34 and velocity 2 for m = 63…66), owner and origin U,
/// level 1, skill 0, the absolute target = the path's target unit's
/// position, else the path target point (+0x10, +0x12); no path → none.
/// PROVISIONAL (settled once slinger5 / slinger6 / catapult1 match the
/// client state): m = 238 / 239 and 406 take their own creators, not
/// modelled (nothing made). The base game makes 239, 222, 141, 238 and
/// 42 there.
///
/// The quill volley after it makes nothing in the base game (aip3 is
/// blank on every BaseId-63 row) and is not run.
pub fn mode_missile(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Result<(), super::dispatch::HandlerError> {
    use super::client_missiles::{create, flag, CreateRecord};
    let Some(m) = w
        .units
        .get(&key)
        .and_then(|u| mode_missile_of(inputs, u, w.tick_rate, w.share_frame()))
    else {
        return Ok(());
    };
    let who = super::critter_path::Who::server(key);
    let target_unit = super::critter_path::target_unit(w, who);
    // 238 / 239 (`pantherpotorange` / `-green`, slinger5 / slinger6):
    // with a target unit, the lob with (U, m, 0, 0, 0, 0, 0, 1, 0)
    // (the zero target takes U's client target position);
    // without one, nothing.
    if matches!(m, 238 | 239) {
        if target_unit.is_some() {
            super::client_do::lob(w, inputs, key, u32::from(m), (0, 0), 0, 1)?;
        }
        return Ok(());
    }
    let target = match target_unit {
        Some(t) => w.units.get(&t).and_then(|t| t.position),
        None => super::critter_path::target(w, who),
    };
    let Some((tx, ty)) = target else {
        return Ok(());
    };
    if m == 406 {
        return Ok(());
    }
    let mut rec = CreateRecord {
        flags: flag::TARGET_ABSOLUTE,
        owner: Some(key),
        origin: Some(key),
        class: u32::from(m),
        tx: i32::from(tx),
        ty: i32::from(ty),
        level: 1,
        ..CreateRecord::default()
    };
    if (63..=66).contains(&m) {
        rec.flags = 0x34;
        rec.velocity = 2;
    }
    create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    Ok(())
}

/// The umod phase-2 hooks of the monster update
/// (after the mode missile step),
/// their client missiles (created at (x, y), skill 0, level 1, owner U): for
/// each of the 9 umod bytes in order, umod 9 `fire` and 31 `goboom` when
/// U is unique (type flag 0x08), in DT (mode 0) and on frame 4 (+0x44 >>
/// 8): at (x, y) = U's sub-tile and the four diagonal points (x + 1, y +
/// 1), (x + 1, y − 1), (x − 1, y + 1), (x − 1, y − 1), five missiles 117
/// `monstercorpseexplode` (umod 9 only), then five 82 `fireexplosion`;
/// umod 40 with no test, missiles 117 then 545 at (x, y).
/// The sounds are the audio layer's (`audio::triggers::hooks`).
pub fn umod_phase2(
    w: &mut ClientWorld,
    inputs: &ModelInputs,
    key: UnitKey,
) -> Result<(), super::dispatch::HandlerError> {
    use super::client_missiles::{create, flag, CreateRecord};
    const CORPSE_EXPLODE: u32 = 117;
    const FIRE_EXPLOSION: u32 = 82;
    const PAIN_WORM: u32 = 545;
    let Some(u) = w.units.get(&key) else {
        return Ok(());
    };
    let super::world::KindData::Monster(m) = &u.kind else {
        return Ok(());
    };
    let (umods, unique) = (m.umods, m.flags & 8 != 0);
    let due = unique && u.mode == 0 && u.frame >> 8 == 4;
    let (x, y) = u.cell();
    let (x, y) = (i32::from(x), i32::from(y));
    let pts = [
        (x, y),
        (x + 1, y + 1),
        (x + 1, y - 1),
        (x - 1, y + 1),
        (x - 1, y - 1),
    ];
    let mut classes: Vec<(u32, (i32, i32))> = Vec::new();
    for umod in umods {
        match umod {
            9 | 31 if due => {
                if umod == 9 {
                    classes.extend(pts.iter().map(|&p| (CORPSE_EXPLODE, p)));
                }
                classes.extend(pts.iter().map(|&p| (FIRE_EXPLOSION, p)));
            }
            40 => classes.extend([(CORPSE_EXPLODE, (x, y)), (PAIN_WORM, (x, y))]),
            _ => {}
        }
    }
    let dir = super::client_do::owner_dir64(w, key);
    for (class, (x, y)) in classes {
        let rec = CreateRecord {
            flags: flag::POSITION,
            owner: Some(key),
            class,
            x,
            y,
            level: 1,
            owner_dir64: dir,
            ..CreateRecord::default()
        };
        create(w, &inputs.tables.missiles, &rec, inputs.high_light_quality)?;
    }
    Ok(())
}

/// The complete test: with a sequence, count < 1; else
/// count ≤ frame + speed (signed 16-bit).
pub fn complete(u: &ClientUnit) -> bool {
    complete_at(u, d2_sim::tick::TickRate::BASE, 0)
}

pub fn complete_at(u: &ClientUnit, rate: d2_sim::tick::TickRate, frame: i32) -> bool {
    if u.sequence.is_some() {
        return u.frame_count < 1;
    }
    u.frame_count
        <= u.frame
            .wrapping_add(rate.share(i32::from(u.speed.unwrap_or(0) as i16), frame))
}

/// The anim step with the record's
/// anim kind: `SplEndDeath` = 1, mode 0 and complete → the machine's code
/// 9 (mode set 12); then kind 0 → advance; 1 → complete → A, else
/// advance; 2 → complete → hold the last frame (+0x44 := +0x48 − 0x100,
/// speed 0; mode 0 also sets unit flag 0x20, no shadow), else advance;
/// 3 → +0x44 := 0x100, speed 0. Returns A.
pub fn anim_step(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, kind: u8) -> bool {
    let (rate, frame) = (w.tick_rate, w.share_frame());
    let complete = |u: &ClientUnit| complete_at(u, rate, frame);
    let advance = |u: &mut ClientUnit| advance_at(u, rate, frame);
    let Some(u) = w.units.get(&key) else {
        return false;
    };
    let end_death = inputs
        .tables
        .monsters
        .get(u.class as usize)
        .and_then(|c| c.as_ref())
        .is_some_and(|c| c.spl_end_death);
    if end_death && u.mode == 0 && complete(u) {
        mode_set(w, inputs, key, 12);
        super::critter_path::server_velocity(w, inputs, key);
    }
    let Some(u) = w.units.get_mut(&key) else {
        return false;
    };
    match kind {
        1 if complete(u) => return true,
        2 if complete(u) => {
            u.frame = u.frame_count - 0x100;
            u.speed = Some(0);
            // The held death animation (mode 0) marks the
            // unit shadowless.
            if u.mode == 0 {
                u.flag_20 = true;
            }
        }
        0..=2 => advance(u),
        3 => {
            u.frame = 0x100;
            u.speed = Some(0);
        }
        _ => {}
    }
    false
}

/// The end test of end kind `kind` from the path
/// step's P and the anim step's A: 1 → P; 2, 3 → A; 4 → P and complete;
/// 0 → never.
pub fn ended(u: &ClientUnit, kind: u8, p: bool, a: bool) -> bool {
    ended_at(u, kind, p, a, d2_sim::tick::TickRate::BASE, 0)
}

pub fn ended_at(
    u: &ClientUnit,
    kind: u8,
    p: bool,
    a: bool,
    rate: d2_sim::tick::TickRate,
    frame: i32,
) -> bool {
    let complete = |u: &ClientUnit| complete_at(u, rate, frame);
    match kind {
        1 => p,
        2 | 3 => a,
        4 => p && complete(u),
        _ => false,
    }
}

/// The mode end of a server monster.
/// KB with BaseId ≠ 78 → path stop and mode set 3. `SplClientEnd` ≠ 0:
/// WL with BaseId 231 / 118, S1 with 110, S4 with 136, DD with 284 and
/// SQ with 247 keep the mode; A1, A2, S1, S2 with 403 / 404 → mode set 8;
/// S3 with 136 → mode set 11. Default: the walk flag 0x2000 set → no
/// change, else mode set 1. Every mode set here restarts the animation
/// (then).
/// Before the `SplClientEnd` switch a unit with a path target faces it
/// (measured).
/// PROVISIONAL: A1 with 497's calls
/// are not run; settled by a capture of
/// such a monster in sight.
pub fn mode_end(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, walk_flag: bool) {
    let Some(u) = w.units.get(&key) else {
        return;
    };
    let mode = u.mode;
    let Some(c) = inputs
        .tables
        .monsters
        .get(u.class as usize)
        .and_then(|c| c.as_ref())
    else {
        return;
    };
    if mode > 15 {
        return;
    }
    let base = c.base_id;
    let restart = |w: &mut ClientWorld, m: u32| {
        if let Some(u) = w.units.get_mut(&key) {
            u.mode_ends = u.mode_ends.wrapping_add(1);
        }
        mode_init(w, inputs, key, m);
        super::critter_path::server_velocity(w, inputs, key);
    };
    if mode == 13 && base != 78 {
        if let Some(u) = w.units.get_mut(&key) {
            if let Some((px, py)) = u.precise {
                u.precise = Some(((px & 0xFFFF_0000) | 0x8000, (py & 0xFFFF_0000) | 0x8000));
            }
        }
        restart(w, 3);
        return;
    }
    // A unit with a target faces it; the
    // next update turns it (a Fallen ending its
    // walk to the player in NU, dir 58 → 56 two ticks later).
    super::critter_path::face_target(w, key);
    if c.spl_client_end {
        let trapped = matches!(base, 403 | 404);
        match (mode, base) {
            (2, 231 | 118) | (8, 110) | (11, 136) | (12, 284) | (14, 247) => return,
            (4 | 5 | 8 | 9, _) if trapped => return restart(w, 8),
            (10, 136) => return restart(w, 11),
            _ => {}
        }
    }
    if !walk_flag {
        restart(w, 1);
    }
}

/// The tail of a pathed mode request: ≠ 0 and
/// the total of stat 67 ≠ → base stat 67 += − total, then the rate
/// again.
pub fn velocity_tail(w: &mut ClientWorld, inputs: &ModelInputs, key: UnitKey, r4: i32) {
    if r4 == 0 {
        return;
    }
    let v = w.total(key, STAT_VELOCITYPERCENT, 0);
    if v == r4 {
        return;
    }
    let base = w.base(key, STAT_VELOCITYPERCENT, 0);
    let Some(u) = w.units.get_mut(&key) else {
        return;
    };
    u.stats
        .insert(STAT_VELOCITYPERCENT, base.wrapping_add(r4 - v));
    let speed = rate(w, inputs, key);
    if let Some(u) = w.units.get_mut(&key) {
        u.speed = Some(speed);
    }
}

#[cfg(test)]
mod tests {
    use super::MonsterSequences;
    use super::*;
    use crate::bridge::world::{ClientUnit, MonsterClass, SkillRow, MONSTER};

    /// Kashya (class 150, `npc`): RCNUHTH 12 frames at 256, RCWLHTH 8 at
    /// 256 (the install's AnimData), walk speed 256; the set-up's stats
    /// 67 = 75, 69 = 100.
    fn kashya() -> (ClientWorld, ModelInputs, UnitKey) {
        let mut inputs = ModelInputs::default();
        let mut c = MonsterClass {
            npc: true,
            walk_speed: 256,
            run_speed: 128,
            ..MonsterClass::default()
        };
        c.anims[1] = Some((12, 256));
        c.anims[2] = Some((8, 256));
        inputs.tables.monsters = vec![None; 151];
        inputs.tables.monsters[150] = Some(c);
        let key = UnitKey::new(MONSTER, 3);
        let mut u = ClientUnit::new(key);
        u.class = 150;
        u.stats.insert(67, 75);
        u.stats.insert(68, 100);
        u.stats.insert(69, 100);
        let mut w = ClientWorld::default();
        w.units.insert(key, u);
        (w, inputs, key)
    }

    #[test]
    fn a_held_death_animation_marks_the_unit_shadowless() {
        // Anim kind 2 holding its last frame sets flag 0x20
        // in mode 0 (DT) only.
        for (mode, want) in [(0, true), (4, false)] {
            let (mut w, inputs, key) = kashya();
            let u = w.units.get_mut(&key).unwrap();
            (u.mode, u.frame_count, u.frame, u.speed) = (mode, 8 << 8, 7 << 8, Some(256));
            anim_step(&mut w, &inputs, key, 2);
            assert_eq!(w.units[&key].frame, 7 << 8, "held, mode {mode}");
            assert_eq!(w.units[&key].flag_20, want, "mode {mode}");
        }
        // Still running: no flag yet.
        let (mut w, inputs, key) = kashya();
        let u = w.units.get_mut(&key).unwrap();
        (u.mode, u.frame_count, u.frame, u.speed) = (0, 8 << 8, 0, Some(256));
        anim_step(&mut w, &inputs, key, 2);
        assert!(!w.units[&key].flag_20);
    }

    #[test]
    fn a_frozen_live_monster_runs_no_update_step() {
        // head: state 1 `freeze` and not dead → return
        // (measured under Frozen Armor: Fallen 1:27 holds frame 7 of GH
        // to f340 in the base game).
        let (mut w, inputs, key) = kashya();
        let u = w.units.get_mut(&key).unwrap();
        (u.mode, u.frame_count, u.frame, u.speed) = (1, 12 << 8, 3 << 8, Some(256));
        u.states.insert(1);
        crate::bridge::critter_path::server_update(&mut w, &inputs, key).unwrap();
        assert_eq!(w.units[&key].frame, 3 << 8, "frozen: no advance");
        w.units.get_mut(&key).unwrap().states.remove(&1);
        crate::bridge::critter_path::server_update(&mut w, &inputs, key).unwrap();
        assert_eq!(w.units[&key].frame, 4 << 8, "thawed: steps again");
        // Dead (mode 12): the gate does not hold it.
        let u = w.units.get_mut(&key).unwrap();
        u.states.insert(1);
        u.mode = 12;
        assert!(!frozen(&w.units[&key]));
        w.units.get_mut(&key).unwrap().mode = 1;
        assert!(frozen(&w.units[&key]));
    }

    #[test]
    fn a_walk_restarts_at_0_and_steps_three_quarters_a_tick() {
        // Seed 1234, arrival: Kashya's walk starts at server tick 32 and
        // her drawn WL frame is floor(0.75·(t − 32)) mod 8 at every tick
        // 33…89 (frame 6 at tick 73).
        let (mut w, inputs, key) = kashya();
        mode_set(&mut w, &inputs, key, 2);
        assert_eq!(w.units[&key].speed, Some(192));
        assert_eq!(w.units[&key].frame_count, 8 << 8);
        for t in 33..=89 {
            anim_step(&mut w, &inputs, key, 0);
            let want = (192 * (t - 32) / 256) % 8;
            assert_eq!(w.units[&key].frame >> 8, want, "tick {t}");
        }
    }

    #[test]
    fn a_unique_fire_enchanted_death_makes_ten_client_missiles() {
        use crate::bridge::client_missiles::ClientMissileRow;
        use crate::bridge::world::{KindData, MonsterData};
        let run = |umods: [u8; 9], flags: u8, mode: u32, frame: i32| {
            let mut inputs = ModelInputs::default();
            inputs.tables.missiles = vec![ClientMissileRow::default(); 546];
            let key = UnitKey::new(MONSTER, 25);
            let mut u = ClientUnit::new(key);
            (u.position, u.mode, u.frame) = (Some((50, 60)), mode, frame);
            u.kind = KindData::Monster(Box::new(MonsterData {
                flags,
                umods,
                ..MonsterData::default()
            }));
            let mut w = ClientWorld::default();
            w.units.insert(key, u);
            umod_phase2(&mut w, &inputs, key).unwrap();
            let mut made: Vec<_> = w
                .objclient
                .set_c
                .values()
                .filter(|u| u.key.unit_type == super::super::world::MISSILE)
                .map(|u| (u.key.guid, u.class, u.cell()))
                .collect();
            made.sort();
            made.into_iter().map(|(_, c, p)| (c, p)).collect::<Vec<_>>()
        };
        let fire = [9, 0, 0, 0, 0, 0, 0, 0, 0];
        let pts = [(50, 60), (51, 61), (51, 59), (49, 61), (49, 59)];
        let want: Vec<_> = [117u32, 82]
            .iter()
            .flat_map(|&c| pts.iter().map(move |&p| (c, p)))
            .collect();
        assert_eq!(run(fire, 8, 0, 0x400), want);
        // A minion, frame 0x3FF, another mode: nothing.
        assert!(run(fire, 0, 0, 0x400).is_empty());
        assert!(run(fire, 8, 0, 0x3FF).is_empty());
        assert!(run(fire, 8, 1, 0x400).is_empty());
        // Goboom: the five 82 only.
        let goboom = [31, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(run(goboom, 8, 0, 0x400), want[5..].to_vec());
        // Worms: no test, 117 then 545 at U.
        let worms = [40, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(run(worms, 0, 1, 0), vec![(117, (50, 60)), (545, (50, 60))]);
    }

    #[test]
    fn a_restart_with_no_net_mode_change_is_counted() {
        // 2 → 1 → 2 within one update: the mode is the same as before,
        // but the walk restarted at frame 0, and the sound pass must see it.
        let (mut w, inputs, key) = kashya();
        mode_set(&mut w, &inputs, key, 2);
        for _ in 0..5 {
            anim_step(&mut w, &inputs, key, 0);
        }
        let before = w.units[&key].anim_restarts;
        mode_set(&mut w, &inputs, key, 1);
        mode_set(&mut w, &inputs, key, 2);
        assert_eq!(w.units[&key].mode, 2);
        assert_eq!(w.units[&key].frame, 0);
        assert_eq!(w.units[&key].anim_restarts, before.wrapping_add(2));
        // The same mode again: no restart, no count.
        mode_set(&mut w, &inputs, key, 2);
        assert_eq!(w.units[&key].anim_restarts, before.wrapping_add(2));
    }

    #[test]
    fn only_a_real_mode_change_is_counted_for_the_mode_sounds() {
        let (mut w, inputs, key) = kashya();
        mode_set(&mut w, &inputs, key, 2);
        let before = w.units[&key].mode_changes;
        // A mode end restarting the unit's own mode: no change.
        mode_init(&mut w, &inputs, key, 2);
        assert_eq!(w.units[&key].mode_changes, before);
        // 2 → 1 → 2: two changes, the mode back where it was.
        mode_init(&mut w, &inputs, key, 1);
        mode_init(&mut w, &inputs, key, 2);
        assert_eq!(w.units[&key].mode_changes, before.wrapping_add(2));
    }

    #[test]
    fn neutral_steps_at_the_animdata_speed() {
        // Recorded: Kashya's NU frame +1 a tick, wrapping at 12.
        let (mut w, inputs, key) = kashya();
        mode_set(&mut w, &inputs, key, 1);
        assert_eq!(w.units[&key].speed, Some(256));
        for t in 1..=30 {
            anim_step(&mut w, &inputs, key, 0);
            assert_eq!(w.units[&key].frame >> 8, t % 12);
        }
    }

    #[test]
    fn the_first_frame_is_a_roll_of_the_count_on_the_unit_seed() {
        let (mut w, inputs, key) = kashya();
        mode_set(&mut w, &inputs, key, 1);
        w.units.get_mut(&key).unwrap().seed = Some((1234, 666));
        first_frame(&mut w, key);
        let mut s = Seed::new(1234, 666);
        let want = s.roll(12 << 8) as i32;
        assert_eq!(w.units[&key].frame, want);
        assert_eq!(w.units[&key].seed, Some((s.lo, s.hi)));
        // No seed in the model: no draw, frame stays.
        w.units.get_mut(&key).unwrap().seed = None;
        first_frame(&mut w, key);
        assert_eq!(w.units[&key].frame, want);
    }

    #[test]
    fn the_pathed_tail_takes_stat_67_from_r4() {
        let (mut w, inputs, key) = kashya();
        mode_set(&mut w, &inputs, key, 2);
        velocity_tail(&mut w, &inputs, key, 100);
        assert_eq!(w.units[&key].stats.get(&67), Some(&100));
        assert_eq!(w.units[&key].speed, Some(256));
        // = 0, or equal to the total: nothing.
        velocity_tail(&mut w, &inputs, key, 0);
        velocity_tail(&mut w, &inputs, key, 100);
        assert_eq!(w.units[&key].stats.get(&67), Some(&100));
    }

    /// A shaman-like class 5: NU 20 frames at 256; skill 47 in slot 0
    /// with `monseq` sequence 1 of 17 frames, all drawn in A2 (mode 5),
    /// frame i at row i, event 1 at row 8 (the Fallen shaman's mode 14
    /// counts 17 frames from 4352).
    fn shaman() -> (ClientWorld, ModelInputs, UnitKey) {
        let mut inputs = ModelInputs::default();
        let mut c = MonsterClass {
            walk_speed: 256,
            run_speed: 256,
            base_id: 5,
            ..MonsterClass::default()
        };
        c.anims[1] = Some((20, 256));
        c.anims[14] = Some((10, 256));
        inputs.tables.monsters = vec![None; 6];
        inputs.tables.monsters[5] = Some(c);
        let mut seq = MonsterSequences {
            slots: vec![[(-1, -1); 8]; 6],
            ..MonsterSequences::default()
        };
        seq.slots[5][0] = (47, 1);
        seq.lists = vec![
            Vec::new(),
            (0..17)
                .map(|i| SeqFrame {
                    mode: 5,
                    frame: i,
                    event: u8::from(i == 8),
                })
                .collect(),
        ];
        inputs.tables.monster_sequences = std::sync::Arc::new(seq);
        let mut skills = vec![SkillRow::default(); 48];
        skills[47].monanim = 14;
        inputs.tables.skills = skills;
        let key = UnitKey::new(MONSTER, 25);
        let mut u = ClientUnit::new(key);
        u.class = 5;
        u.mode = 1;
        let mut w = ClientWorld::default();
        w.units.insert(key, u);
        (w, inputs, key)
    }

    #[test]
    fn a_cast_request_sets_mode_14_and_loads_the_sequence() {
        let (mut w, inputs, key) = shaman();
        let out = crate::bridge::output::Outputs::default();
        crate::bridge::modes::mode_request(
            &mut w,
            &inputs,
            key,
            0x15,
            [47, 0, 330, 190, 1, 0, 0],
            &out,
        )
        .unwrap();
        let u = &w.units[&key];
        assert_eq!(u.mode, 14);
        assert_eq!(u.used_skill, Some(47));
        assert_eq!(u.frame_count, 17 << 8);
        assert_eq!(u.frame, 0);
        let seq = u.sequence.as_ref().expect("loaded");
        assert_eq!(
            (seq.length, seq.pos, seq.step, seq.drawn_mode),
            (17 << 8, 0, 256, 5)
        );
    }

    #[test]
    fn the_sequence_counts_down_and_ends_in_neutral() {
        // A Fire Ball shaman (GUID 25): mode 14 drawn as A2,
        // frame 15 with 512 left, then frame 0 with 0 left; one update
        // later NU frame 0, then NU at its rate.
        let (mut w, inputs, key) = shaman();
        w.units.get_mut(&key).unwrap().used_skill = Some(47);
        mode_set(&mut w, &inputs, key, 14);
        let mut events = Vec::new();
        for t in 1..=17 {
            crate::bridge::critter_path::server_update(&mut w, &inputs, key).unwrap();
            let u = &w.units[&key];
            assert_eq!(u.mode, 14, "tick {t}");
            assert_eq!(u.frame_count, (17 - t) << 8);
            assert_eq!(u.frame, (t % 17) << 8);
            assert_eq!(u.sequence.as_ref().unwrap().drawn_mode, 5);
            events.push(u.action_frame);
        }
        assert_eq!(
            events.iter().position(|&e| e == 1),
            Some(7),
            "row 8's event"
        );
        crate::bridge::critter_path::server_update(&mut w, &inputs, key).unwrap();
        let u = &w.units[&key];
        assert_eq!((u.mode, u.frame, u.frame_count), (1, 0, 20 << 8));
        assert!(u.sequence.is_none());
        // Then NU advances by its rate (256 a tick for the real shaman;
        // this fixture's rate input gives its own speed).
        let speed = w.units[&key].speed.expect("rate set");
        crate::bridge::critter_path::server_update(&mut w, &inputs, key).unwrap();
        assert_eq!(w.units[&key].frame, speed);
    }

    #[test]
    fn a_moving_skill_entry_runs_the_do_when_its_path_is_done() {
        // Entry bit 0 and the path done (mode 14, P = 1) →
        // flags |= 2 unless `srvdofunc` 67, then the do (flag 0x40).
        for (srvdofunc, flags, byte, want_flags, want_do) in [
            (0, 1, 0, 3, true),
            (67, 1, 0, 1, true),
            (0, 0, 0, 0, false),
            (0, 0, 1, 0, true),
        ] {
            let (mut w, mut inputs, key) = shaman();
            inputs.tables.skills[47].srvdofunc = srvdofunc;
            let u = w.units.get_mut(&key).unwrap();
            u.mode = 14;
            u.used_skill = Some(47);
            u.action_frame = byte;
            let mut list = crate::bridge::skills::SkillList::default();
            list.entries.push(crate::bridge::skills::SkillEntry {
                skill: 47,
                flags,
                ..Default::default()
            });
            list.current = Some(0);
            u.skills = Some(list);
            skill_step(&mut w, &inputs, key).unwrap();
            let e = crate::bridge::skills::used_entry(&w, key).unwrap();
            assert_eq!(e.flags, want_flags, "srvdofunc {srvdofunc} flags {flags}");
            assert_eq!(
                w.units[&key].flag_40, want_do,
                "srvdofunc {srvdofunc} flags {flags}"
            );
        }
    }

    #[test]
    fn the_walk_flag_keeps_the_ended_mode() {
        let (mut w, inputs, key) = shaman();
        w.units.get_mut(&key).unwrap().used_skill = Some(47);
        mode_set(&mut w, &inputs, key, 14);
        w.units.get_mut(&key).unwrap().flag_ex |= 0x2000;
        for _ in 0..20 {
            crate::bridge::critter_path::server_update(&mut w, &inputs, key).unwrap();
        }
        assert_eq!(w.units[&key].mode, 14);
    }

    #[test]
    fn mode_records_and_the_held_last_frame() {
        let (mut w, mut inputs, key) = shaman();
        assert_eq!(mode_record(&inputs, 5, 13), Some((1, 2, 2)));
        assert_eq!(mode_record(&inputs, 5, 4), Some((2, 1, 3)));
        assert_eq!(mode_record(&inputs, 5, 16), None);
        assert_eq!(mode_record(&inputs, 4, 1), None, "no class row");
        inputs.tables.monsters[5].as_mut().unwrap().move_modes = 1 << 4;
        assert_eq!(mode_record(&inputs, 5, 4), Some((1, 0, 4)));
        inputs.tables.monsters[5].as_mut().unwrap().base_id = 78;
        assert_eq!(mode_record(&inputs, 5, 0), Some((1, 2, 0)));
        assert_eq!(mode_record(&inputs, 243, 11), None, "no row 243");
        // Anim kind 2 holds the last frame once complete.
        inputs.tables.monsters[5].as_mut().unwrap().anims[12] = Some((4, 256));
        mode_set(&mut w, &inputs, key, 12);
        for _ in 0..10 {
            assert!(!anim_step(&mut w, &inputs, key, 2));
        }
        let u = &w.units[&key];
        assert_eq!((u.frame, u.speed), (3 << 8, Some(0)));
    }

    #[test]
    fn a_knockback_end_sets_get_hit() {
        let (mut w, inputs, key) = shaman();
        mode_set(&mut w, &inputs, key, 13);
        mode_end(&mut w, &inputs, key, false);
        assert_eq!(w.units[&key].mode, 3);
    }

    #[test]
    fn the_dead_snap_reads_the_cof_direction_count() {
        // Mephisto (class 242) DD: `d<mode>` 8, its COF 16 directions:
        // ready, n = 8 ≠ D = 16 → the (16, 8) snap (dir64 >> 2 = 9 through
        // 0, 0, 2, 2, …, 8, 8, …: 8 << 2): 37 → 32. Not ready, n = 4: (16, 4) has no
        // table, so the direction is written back unchanged.
        let mut inputs = ModelInputs::default();
        let mut c = MonsterClass::default();
        c.directions[12] = 8;
        c.cof_dirs[12] = 16;
        let mut one = MonsterClass::default();
        one.directions[0] = 8;
        one.cof_dirs[0] = 1;
        inputs.tables.monsters = vec![None; 502];
        inputs.tables.monsters[242] = Some(c);
        inputs.tables.monsters[501] = Some(one);
        let key = UnitKey::new(MONSTER, 7);
        let mut u = ClientUnit::new(key);
        (u.class, u.mode, u.path_dir) = (242, 12, Some(37));
        let mut w = ClientWorld::default();
        w.units.insert(key, u);
        let mut ready = crate::bridge::world::GfxReady::default();
        ready.masks.insert(242, 1 << 12);
        let mut map = std::collections::BTreeMap::new();
        map.insert(0, ready);
        w.gfx_ready = Some(std::sync::Arc::new(map));
        death_snap(&mut w, &inputs);
        assert_eq!(w.units[&key].path_dir, Some(32));
        w.gfx_ready = None;
        w.units.get_mut(&key).unwrap().path_dir = Some(37);
        death_snap(&mut w, &inputs);
        assert_eq!(w.units[&key].path_dir, Some(37));
        // A one-direction DT COF (d 8, D 1; class 501): never snapped.
        let u = w.units.get_mut(&key).unwrap();
        (u.class, u.mode, u.path_dir) = (501, 0, Some(37));
        death_snap(&mut w, &inputs);
        assert_eq!(w.units[&key].path_dir, Some(37));
    }

    #[test]
    fn a_dead_fallen_snaps_32_to_40() {
        // A Fallen kill: guid 19 dies with d 32
        // (S→C 0x69 code 8); the client's path direction reads 40 from the
        // next frame (d<mode> 8, 4 directions drawn: 1,1,3,3,5,5,7,7).
        let mut inputs = ModelInputs::default();
        let mut c = MonsterClass::default();
        c.directions[0] = 8;
        c.directions[12] = 8;
        inputs.tables.monsters = vec![None; 20];
        inputs.tables.monsters[19] = Some(c);
        let key = UnitKey::new(MONSTER, 19);
        let mut u = ClientUnit::new(key);
        u.class = 19;
        u.mode = 12;
        u.path_dir = Some(32);
        let mut w = ClientWorld::default();
        w.units.insert(key, u);
        death_snap(&mut w, &inputs);
        assert_eq!(w.units[&key].path_dir, Some(40));
        // Alive (mode 1): no snap.
        let u = w.units.get_mut(&key).unwrap();
        u.mode = 1;
        u.path_dir = Some(32);
        death_snap(&mut w, &inputs);
        assert_eq!(w.units[&key].path_dir, Some(32));
        // A stored ready flag for DD (bit 12) from tick 5 on: dead at
        // tick 4 it snaps, at tick 5 it keeps its direction.
        let u = w.units.get_mut(&key).unwrap();
        u.mode = 12;
        u.path_dir = Some(33);
        let mut ready = crate::bridge::world::GfxReady::default();
        ready.masks.insert(19, 1 << 12);
        let mut map = std::collections::BTreeMap::new();
        map.insert(2, crate::bridge::world::GfxReady::default());
        map.insert(5, ready);
        w.gfx_ready = Some(std::sync::Arc::new(map));
        w.server_ticks = 4;
        death_snap(&mut w, &inputs);
        assert_eq!(w.units[&key].path_dir, Some(40));
        w.units.get_mut(&key).unwrap().path_dir = Some(33);
        w.server_ticks = 6;
        death_snap(&mut w, &inputs);
        assert_eq!(w.units[&key].path_dir, Some(33));
    }

    #[test]
    fn the_first_direction_follows_the_base_class_rules() {
        // Mummy (BaseId 96) d 7, a vile
        // child (BaseId 301) with no mother beside it d 0, whatever the
        // seed step gives; a child born in front of its mother (3 cells
        // along her facing) takes her direction.
        let row = |base_id: u16| MonsterClass {
            base_id,
            modes: 1 << 2,
            size_x: 2,
            ..MonsterClass::default()
        };
        let mut monsters = vec![None; 302];
        monsters[96] = Some(row(96));
        monsters[298] = Some(row(298));
        monsters[301] = Some(row(301));
        let new = |w: &mut ClientWorld, guid: u32, class: u32, at: (u16, u16)| {
            let key = UnitKey::new(MONSTER, guid);
            let mut u = ClientUnit::new(key);
            (u.class, u.position, u.seed) = (class, Some(at), Some((1, 2)));
            w.units.insert(key, u);
            key
        };
        let mut w = ClientWorld::default();
        let mummy = new(&mut w, 1, 96, (100, 100));
        first_direction(&mut w, &monsters, mummy, monsters[96].as_ref().unwrap());
        assert_eq!(w.units[&mummy].path_dir, Some(7));
        let child = new(&mut w, 2, 301, (200, 200));
        first_direction(&mut w, &monsters, child, monsters[301].as_ref().unwrap());
        assert_eq!(w.units[&child].path_dir, Some(0));
        // A mother at (300, 300) facing 16 (dir8 2 -> pair 0 = (0, -1)):
        // her birth spot is (300, 297).
        let mother = new(&mut w, 3, 298, (300, 300));
        w.units.get_mut(&mother).unwrap().path_dir = Some(16);
        let born = new(&mut w, 4, 301, (300, 297));
        first_direction(&mut w, &monsters, born, monsters[301].as_ref().unwrap());
        assert_eq!(w.units[&born].path_dir, Some(16));
        let beside = new(&mut w, 5, 301, (301, 300));
        first_direction(&mut w, &monsters, beside, monsters[301].as_ref().unwrap());
        assert_eq!(w.units[&beside].path_dir, Some(0), "not on the birth spot");
    }

    #[test]
    fn a_monster_dead_off_screen_keeps_its_direction() {
        // A Fallen killed 40 sub-tiles
        // east of the player keeps d 20 (no draw, no write-back); on
        // screen it snaps as drawn.
        let mut inputs = ModelInputs::default();
        let mut c = MonsterClass::default();
        c.directions[0] = 8;
        inputs.tables.monsters = vec![None; 20];
        inputs.tables.monsters[19] = Some(c);
        let key = UnitKey::new(MONSTER, 8);
        let mut u = ClientUnit::new(key);
        (u.class, u.mode, u.path_dir) = (19, 0, Some(20));
        u.position = Some((5183, 4263));
        let mut w = ClientWorld::default();
        w.units.insert(key, u);
        // The camera of a player at (5143, 4263), 800 x 600.
        let player =
            crate::rules::camera::moving_to_client((5143 << 16) | 0x8000, (4263 << 16) | 0x8000);
        let cam = crate::rules::camera::Camera::new(
            crate::rules::camera::FrameSize::D2RUST,
            crate::rules::camera::OpenMode::NONE,
            player,
            (0, 0),
        );
        w.unit_origin = Some(crate::bridge::world::UnitOrigin::of(&cam));
        death_snap(&mut w, &inputs);
        assert_eq!(w.units[&key].path_dir, Some(20), "off screen: kept");
        w.units.get_mut(&key).unwrap().position = Some((5153, 4263));
        death_snap(&mut w, &inputs);
        assert_eq!(w.units[&key].path_dir, Some(24), "on screen: snapped");
    }
}
