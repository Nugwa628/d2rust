//! The server-do and server-hit function tables and their dispatch.
//! The id tables below are checked against the function catalogues by
//! `tests::catalogues_match_tsv`.
//!
//! Every non-null entry has a body. Server-do 1 is the default flight in
//! [`super::flight`]; the other bodies are in [`super::bodies`],
//! [`super::bodies_ext`] and [`super::bodies_ext2`]; the seeded
//! sub-missile helper that server-do 8, 10, 17 and 25 call is here. A
//! null entry logs [`Unhandled`] and does nothing. A mod game's bodies
//! ([`super::MissileProvider`]) answer first, and its slots past the
//! tables are called too ([`srv_do_called`], [`srv_hit_called`]).

use crate::game::Game;
use crate::units::UnitId;

use super::create::{create_missile, MissileParams};
use super::flight::default_flight;
use super::provider::MissileSlot;
use super::{param_flags, Ctx, MissileWorld, Unhandled, SRV_DO_COUNT, SRV_HIT_COUNT};

/// The server-do function table (53 entries).
pub const SRV_DO: [Option<u32>; SRV_DO_COUNT as usize] = [
    None,
    Some(0x005B0BC0),
    Some(0x005AE400),
    Some(0x005AE480),
    None,
    Some(0x005AE520),
    Some(0x005AE680),
    Some(0x005AE780),
    Some(0x005AE8A0),
    Some(0x005AE940),
    Some(0x005AEA60),
    Some(0x005AEB60),
    Some(0x005AECA0),
    Some(0x005AEDA0),
    Some(0x005AEF70),
    Some(0x005AF030),
    Some(0x005AF170),
    Some(0x005AF240),
    Some(0x005AF300),
    Some(0x005B0940),
    Some(0x005AF540),
    Some(0x005AF590),
    Some(0x005AF620),
    Some(0x005AF790),
    Some(0x005AF790),
    Some(0x005AF880),
    Some(0x005AF980),
    Some(0x005AFA30),
    Some(0x005AFB80),
    Some(0x005AFD70),
    Some(0x005B0010),
    Some(0x005B01F0),
    Some(0x005B03E0),
    Some(0x005AFEC0),
    Some(0x005B04A0),
    Some(0x005B0640),
    Some(0x005B0A40),
    Some(0x005B0AA0),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
];

/// The server-hit function table (71 entries).
pub const SRV_HIT: [Option<u32>; SRV_HIT_COUNT as usize] = [
    None,
    Some(0x005A9A70),
    Some(0x005A9D80),
    Some(0x005A9F90),
    Some(0x005B07A0),
    Some(0x005ABC40),
    Some(0x005AA1C0),
    Some(0x005A9FB0),
    Some(0x005AA180),
    Some(0x005AA250),
    Some(0x005AA650),
    Some(0x005B0870),
    Some(0x005AA730),
    Some(0x005AA8B0),
    Some(0x005AABB0),
    Some(0x005AAD40),
    Some(0x005AAE10),
    Some(0x005AAFB0),
    Some(0x005AB0B0),
    Some(0x005AB110),
    Some(0x005AB370),
    Some(0x005AB500),
    Some(0x005ADD20),
    Some(0x005ACFC0),
    Some(0x005A9BF0),
    Some(0x005AB820),
    Some(0x005AB8D0),
    Some(0x005ABA00),
    Some(0x005ABA10),
    Some(0x005ABB00),
    None,
    Some(0x005ABD70),
    Some(0x005ABE50),
    Some(0x005ABEB0),
    None,
    Some(0x005ABEE0),
    Some(0x005ABF70),
    Some(0x005AC020),
    Some(0x005AC0A0),
    Some(0x005AC1D0),
    Some(0x005AC250),
    None,
    None,
    Some(0x005AC350),
    Some(0x005A9E10),
    Some(0x005AC480),
    None,
    Some(0x005AC550),
    Some(0x005AC6D0),
    None,
    Some(0x005AC800),
    Some(0x005AC870),
    Some(0x005AC940),
    Some(0x005ACA50),
    Some(0x005ACAF0),
    Some(0x005ACB60),
    Some(0x005ACC50),
    Some(0x005AD970),
    Some(0x005ACDF0),
    Some(0x005ACF20),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
];

/// The catalogue texts, for the consistency test.
pub const SRVDO_TSV: &str = include_str!("../../../../tables/missiles/srvdo.tsv");
pub const SRVHIT_TSV: &str = include_str!("../../../../tables/missiles/srvhit.tsv");

/// Server-do indices with a body here.
pub const SRV_DO_IMPLEMENTED: [i16; 36] = [
    1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
    28, 29, 30, 31, 32, 33, 34, 35, 36, 37,
];
/// Server-hit indices with a body here.
pub const SRV_HIT_IMPLEMENTED: [i16; 53] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 31, 32, 33, 35, 36, 37, 38, 39, 40, 43, 44, 45, 47, 48, 50, 51, 52, 53, 54, 55, 56,
    57, 58, 59,
];

/// Whether a server-hit index is called (1…70).
pub fn srv_hit_in_range(index: i16) -> bool {
    (1..SRV_HIT_COUNT).contains(&index)
}

/// Whether the class handler calls server-do `index`: D2RUST's 1…52,
/// or a slot a mod game has ([`super::MissileProvider::has`]).
pub fn srv_do_called<W: MissileWorld + ?Sized>(cx: &Ctx<'_, W>, index: i16) -> bool {
    (1..SRV_DO_COUNT).contains(&index) || mod_has(cx, MissileSlot::Do, index)
}

/// Whether the hit handler calls server-hit `index`: D2RUST's 1…70, or a
/// slot a mod game has ([`super::MissileProvider::has`]).
pub fn srv_hit_called<W: MissileWorld + ?Sized>(cx: &Ctx<'_, W>, index: i16) -> bool {
    srv_hit_in_range(index) || mod_has(cx, MissileSlot::Hit, index)
}

fn mod_has<W: MissileWorld + ?Sized>(cx: &Ctx<'_, W>, slot: MissileSlot, index: i16) -> bool {
    cx.world
        .rules()
        .missiles()
        .is_some_and(|p| p.has(slot, index))
}

/// Runs server-do `index` (the caller checked [`srv_do_called`]). A mod
/// game's body ([`super::MissileProvider::srv_do`]) answers first.
pub fn run_srv_do<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    index: i16,
    m: UnitId,
) -> i32 {
    if !cx.world.rules().is_original() {
        let rules = cx.world.rules().clone();
        if let Some(r) = rules
            .missiles()
            .and_then(|p| p.srv_do(game, &mut cx.as_dyn(), index, m))
        {
            return r;
        }
    }
    use super::bodies::*;
    use super::bodies_ext::*;
    use super::bodies_ext2::*;
    match index {
        1 => default_flight(game, cx, m),
        2 => srv_do_2(game, cx, m),
        3 => srv_do_3(game, cx, m),
        5 => srv_do_5(game, cx, m),
        6 => srv_do_6(game, cx, m),
        7 => srv_do_7(game, cx, m),
        8 => srv_do_8(game, cx, m),
        9 => srv_do_9(game, cx, m),
        10 => srv_do_10(game, cx, m),
        11 => srv_do_11(game, cx, m),
        12 => srv_do_12(game, cx, m),
        13 => srv_do_13(game, cx, m),
        14 => srv_do_14(game, cx, m),
        15 => srv_do_15(game, cx, m),
        16 => srv_do_16(game, cx, m),
        17 => srv_do_17(game, cx, m),
        18 => srv_do_18(game, cx, m),
        19 => srv_do_19(game, cx, m),
        20 => srv_do_20(game, cx, m),
        21 => srv_do_21(game, cx, m),
        22 => srv_do_22(game, cx, m),
        // One function at both entries.
        23 | 24 => srv_do_23(game, cx, m),
        25 => srv_do_25(game, cx, m),
        26 => srv_do_26(game, cx, m),
        27 => srv_do_27(game, cx, m),
        28 => srv_do_28(game, cx, m),
        29 => srv_do_29(game, cx, m),
        30 => srv_do_30(game, cx, m),
        31 => srv_do_31(game, cx, m),
        32 => srv_do_32(game, cx, m),
        33 => srv_do_33(game, cx, m),
        34 => srv_do_34(game, cx, m),
        35 => srv_do_35(game, cx, m),
        36 => srv_do_36(game, cx, m),
        37 => srv_do_37(game, cx, m),
        _ => {
            let entry = SRV_DO.get(index as usize).copied().flatten();
            cx.store.unhandled.push(match entry {
                Some(_) => Unhandled::SrvDo { index, missile: m },
                None => Unhandled::NullSrvDo { index, missile: m },
            });
            // Only null entries reach here (a fatal case): keeps the
            // missile, does nothing else.
            1
        }
    }
}

/// Runs server-hit `index` with the hit handler's current result `c`
/// (the caller checked [`srv_hit_called`]). A mod game's body
/// ([`super::MissileProvider::srv_hit`]) answers first, then the bodies
/// of [`super::bodies`], [`super::bodies_ext`] and [`super::bodies_ext2`];
/// an empty entry returns `c` unchanged.
pub fn run_srv_hit<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    index: i16,
    m: UnitId,
    unit: Option<UnitId>,
    c: i32,
) -> i32 {
    if !cx.world.rules().is_original() {
        let rules = cx.world.rules().clone();
        if let Some(r) = rules
            .missiles()
            .and_then(|p| p.srv_hit(game, &mut cx.as_dyn(), index, m, unit, c))
        {
            return r;
        }
    }
    use super::bodies::*;
    use super::bodies_ext::*;
    use super::bodies_ext2::*;
    match index {
        1 => srv_hit_1(game, cx, m, unit),
        2 => srv_hit_2(game, cx, m, unit),
        3 => srv_hit_3(game, cx, m, unit),
        4 => srv_hit_4(game, cx, m, unit),
        5 => srv_hit_5(game, cx, m),
        6 => srv_hit_6(game, cx, m),
        7 => srv_hit_7(game, cx, m, unit),
        8 => srv_hit_8(game, cx, m, unit),
        9 => srv_hit_9(game, cx, m, unit),
        10 => srv_hit_10(game, cx, m, unit),
        11 => srv_hit_11(game, cx, m, unit),
        12 => srv_hit_12(game, cx, m, unit),
        13 => srv_hit_13(game, cx, m, unit),
        14 => srv_hit_14(game, cx, m, unit),
        15 => srv_hit_15(game, cx, m),
        16 => srv_hit_16(game, cx, m, unit),
        17 => srv_hit_17(game, cx, m, unit),
        18 => srv_hit_18(game, cx, m, unit),
        19 => srv_hit_19(game, cx, m, unit),
        20 => srv_hit_20(game, cx, m),
        21 => srv_hit_21(game, cx, m, unit),
        22 => srv_hit_22(game, cx, m),
        23 => srv_hit_23(game, cx, m, unit),
        24 => srv_hit_24(game, cx, m, unit),
        25 => srv_hit_25(game, cx, m, unit),
        26 => srv_hit_26(game, cx, m),
        // Returns 1 and does nothing else.
        27 => 1,
        28 => srv_hit_28(game, cx, m, unit),
        29 => srv_hit_29(game, cx, m),
        31 => srv_hit_31(game, cx, m, unit),
        32 => srv_hit_32(game, cx, m, unit),
        33 => srv_hit_33(game, cx, m, unit),
        35 => srv_hit_35(game, cx, m, unit),
        36 => srv_hit_36(game, cx, m, unit),
        37 => srv_hit_37(unit),
        38 => srv_hit_38(game, cx, m),
        39 => srv_hit_39(game, cx, m),
        40 => srv_hit_40(game, cx, m),
        43 => srv_hit_43(game, cx, m, unit),
        44 => srv_hit_44(game, cx, m, unit),
        45 => srv_hit_45(game, cx, m),
        47 => srv_hit_47(game, cx, m, unit),
        48 => srv_hit_48(game, cx, m),
        50 => srv_hit_50(game, cx, m, unit),
        51 => srv_hit_51(game, cx, m),
        52 => srv_hit_52(game, cx, m),
        53 => srv_hit_53(game, cx, m, unit),
        54 => srv_hit_54(game, cx, m, unit),
        55 => srv_hit_55(game, cx, m, unit),
        56 => srv_hit_56(game, cx, m),
        57 => srv_hit_57(game, cx, m, unit),
        58 => srv_hit_58(game, cx, m),
        59 => srv_hit_59(game, cx, m),
        _ => null_srv_hit(cx, index, m, c),
    }
}

/// A null server-hit entry (a fatal case): logged, as if the row had no
/// function.
fn null_srv_hit<W: MissileWorld + ?Sized>(
    cx: &mut Ctx<'_, W>,
    index: i16,
    m: UnitId,
    c: i32,
) -> i32 {
    let entry = SRV_HIT.get(index as usize).copied().flatten();
    cx.store.unhandled.push(match entry {
        Some(_) => Unhandled::SrvHit { index, missile: m },
        None => Unhandled::NullSrvHit { index, missile: m },
    });
    // Only null entries reach here (a fatal case): as if the row had no
    // function.
    c
}

/// Every `interval` base ticks of elapsed lifetime, re-seed the missile with
/// `init_low(x + elapsed)`, draw an offset within `range` and create
/// `class` there unless the collision there matches `mask`.
pub fn create_with_collision_check<W: MissileWorld + ?Sized>(
    game: &mut Game,
    cx: &mut Ctx<'_, W>,
    m: UnitId,
    range: i32,
    interval: i32,
    class: i32,
    mask: u16,
) -> Option<UnitId> {
    let d = cx.store.get(m)?;
    let elapsed = d.elapsed();
    let (skill, level) = (i32::from(d.skill), i32::from(d.level));
    // One sub-missile every `interval` base ticks of the missile's life
    // (signed remainder; an interval of 0 makes none).
    let interval = game.ticks(interval);
    if interval == 0 || elapsed % interval != 0 {
        return None;
    }
    // Step 2.
    let owner = cx.owner(game, m)?;
    let room = game.lists.unit(m)?.room()?;
    // Steps 3–4.
    let (x, y) = cx.world.position(m);
    let (dx, dy) = seeded_offset(cx.world.seed(m), x, elapsed, range);
    let (sx, sy) = (x.wrapping_add(dx), y.wrapping_add(dy));
    // Step 5.
    if cx.world.collision_at(game, room, sx, sy, mask) != 0 {
        return None;
    }
    let p = MissileParams {
        flags: param_flags::POSITION | param_flags::TARGET_RELATIVE,
        owner: Some(owner),
        class,
        x: sx,
        y: sy,
        skill,
        level,
        ..MissileParams::default()
    };
    create_missile(game, cx, &p)
}

/// Re-seed `{x + elapsed, 666}`, then dx, dy =
/// `roll(2(r − 1)) − (r − 1)` each, in that order.
pub fn seeded_offset(seed: &mut crate::rng::Seed, x: i32, elapsed: i32, range: i32) -> (i32, i32) {
    *seed = crate::rng::Seed::init_low(x.wrapping_add(elapsed) as u32);
    let r1 = range.wrapping_sub(1);
    let dx = (seed.roll(r1.wrapping_mul(2)) as i32).wrapping_sub(r1);
    let dy = (seed.roll(r1.wrapping_mul(2)) as i32).wrapping_sub(r1);
    (dx, dy)
}
