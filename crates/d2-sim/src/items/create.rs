//! The creation pipeline, base stats, elixirs and quest items,
//! the normal-quality routine and class skill mods, sockets,
//! ethereal, forced requests, ears and replenish timers,
//! and the format-0 normal routine and class skill mods.

use super::quality::dispatch;
use super::tables::ItemTables;
use super::{
    flag, q, req, stat, ty, Fatal, Item, ItemGame, ItemRequest, ItemStats, ListKey, PlayerInfo,
};
use crate::rng::Seed;

/// Why creation produced no item (Outputs: the unit is
/// removed; seed steps already made stay made).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CreateError {
    /// Classic game and a missing or expansion-only record.
    #[error("classic game: no item record or version ≥ 100")]
    Classic,
    /// Item index out of range (nothing allocated).
    #[error("item index out of range")]
    BadIndex,
    /// The quality dispatch returned 0.
    #[error("quality dispatch failed")]
    Failed,
    /// An ear's request unit is not a player with player data.
    #[error("ear without a player")]
    NotPlayer,
    /// The base game exits the process here; D2RUST returns the error.
    #[error(transparent)]
    Fatal(#[from] Fatal),
}

/// A created item and the frame of its replenish timer (event 3), if one
/// is due (scheduling is the caller's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Created<S> {
    pub item: Item<S>,
    pub event3_at: Option<u32>,
}

/// The elixir file-index table.
pub const ELIXIRS: [i32; 6] = [0, 1, 2, 3, 9, 7];

/// The pipeline (D2MOO `D2GAME_CreateItemEx`).
///
/// `stats` is the new unit's stat holder (allocation is the caller's);
/// the seeds are derived here from the game seed in the
/// allocation's order. `frame` is the current game frame (for the
/// replenish timer). The request's `ilvl` and `quality` are written back
/// as the base game does.
pub fn create_item<S: ItemStats>(
    t: &ItemTables,
    game: &mut dyn ItemGame,
    rq: &mut ItemRequest,
    use_seed: bool,
    stats: S,
    frame: u32,
) -> Result<Created<S>, CreateError> {
    let rec = usize::try_from(rq.item)
        .ok()
        .and_then(|i| t.item(i).map(|r| (i, r)));
    if !game.expansion() && rec.is_none_or(|(_, r)| r.version >= 100) {
        return Err(CreateError::Classic);
    }
    let Some((idx, _)) = rec else {
        return Err(CreateError::BadIndex);
    };
    // Unit seed, then the item seed ({1, 666} then one game step).
    let mut item = Item::new(idx, rq.format, stats);
    // The item data is zero-filled, so the file
    // index starts at 0 (only the quality routines write it).
    item.file_index = 0;
    item.unit_seed = game.seed().derive();
    item.init_seed = item.unit_seed.lo;
    item.start_seed = game.seed().step();
    item.item_seed = Seed::init_low(item.start_seed);
    // The item kind's init sets 0x10 at allocation; only a successful
    // quality routine clears it.
    item.flags |= flag::INIT | flag::IDENTIFIED;
    if use_seed || rq.force {
        item.unit_seed = Seed::init_low(rq.seed);
        item.init_seed = rq.seed;
        item.start_seed = rq.item_seed;
        item.item_seed = Seed::init_low(rq.item_seed);
    }
    if rq.ilvl < 1 {
        rq.ilvl = 1;
    }
    item.ilvl = rq.ilvl;
    if !rq.force {
        item.flags |= flag::INSTORE;
    }
    item.inv_page = 0xFF;
    if !init_item_stats(t, game, &mut item, Some(rq), true)? {
        return Err(CreateError::Failed);
    }
    if let Some(f) = item.fatal {
        return Err(f.into());
    }
    if rq.force {
        forced(t, &mut item, rq);
    }
    let primary = t.item(idx).map_or(-1, |r| r.type_);
    if primary == ty::PLAY as i16 {
        ear(&mut item, rq)?;
    }
    if item.flags & flag::PERSONALIZED != 0 {
        personalize(&mut item, rq)?;
    }
    let event3_at = replenish_timer_at(&item.stats, false, frame, game.tick_rate());
    crate::cov!(Item, item.record, item.quality);
    Ok(Created { item, event3_at })
}

/// Create from an index: a request
/// with the unit, item, quality, the game's item format and the seeds
/// (allocation fields live with the caller), `flags2` NO_SOCKETS /
/// NEVER_ETHEREAL from the two switches, `ilvl` ≤ 0 becoming 1; then the
/// pipeline with `use_seed`, and on success the identified flag.
#[allow(clippy::too_many_arguments)]
pub fn create_from_index<S: ItemStats>(
    t: &ItemTables,
    game: &mut dyn ItemGame,
    unit: Option<super::RequestUnit>,
    index: i32,
    quality: u8,
    no_sockets: bool,
    never_ethereal: bool,
    ilvl: i32,
    use_seed: bool,
    seed: u32,
    item_seed: u32,
    stats: S,
    frame: u32,
) -> Result<Created<S>, CreateError> {
    let mut rq = ItemRequest {
        unit,
        item: index,
        quality,
        format: game.item_format(),
        ilvl: if ilvl <= 0 { 1 } else { ilvl },
        seed,
        item_seed,
        ..Default::default()
    };
    if no_sockets {
        rq.flags2 |= req::NO_SOCKETS;
    }
    if never_ethereal {
        rq.flags2 |= req::NEVER_ETHEREAL;
    }
    let mut created = create_item(t, game, &mut rq, use_seed, stats, frame)?;
    created.item.flags |= flag::IDENTIFIED;
    Ok(created)
}

/// `roll(n)` on the unit seed as i32.
fn r(seed: &mut Seed, n: i32) -> i32 {
    seed.roll(n) as i32
}

/// "Total max stack": `maxstack` + stat 254, capped at 511.
fn total_max_stack<S: ItemStats>(t: &ItemTables, item: &Item<S>) -> i32 {
    let m = t.item(item.record).map_or(0, |r| r.maxstack as i32);
    m.wrapping_add(item.stats.stat(stat::EXTRA_STACK, 0))
        .min(511)
}

/// Base stats (D2MOO `D2GAME_InitItemStats`). With
/// `quest` and a request it runs the quality dispatch and returns its
/// result; else `true`.
pub fn init_item_stats<S: ItemStats>(
    t: &ItemTables,
    game: &mut dyn ItemGame,
    item: &mut Item<S>,
    rq: Option<&mut ItemRequest>,
    quest: bool,
) -> Result<bool, Fatal> {
    let rec = t.item(item.record).cloned();
    let primary = rec.as_ref().map_or(-1, |r| r.type_);
    let itype = t.itype_of(item.record).cloned();
    let ovr = rq.as_ref().map_or(0, |r| r.quantity_override);
    if primary == ty::GOLD as i16 {
        let g = match rq.as_deref() {
            Some(_) => {
                let ilvl = item.item_level();
                // TODO(items OQ-G1): "a drop-request quantity > 0" read as
                // the quantity override (+0x54).
                let mut g = r(&mut item.unit_seed, 5i32.wrapping_mul(ilvl)) + ilvl;
                if g <= 0 {
                    g = 1;
                }
                if ovr > 0 {
                    g = ovr;
                }
                g
            }
            None => 1,
        };
        set_gold(item, g);
    } else if itype.as_ref().is_some_and(|it| it.quiver != 0) {
        let min = rec.as_ref().map_or(0, |r| r.minstack as i32);
        let n0 = total_max_stack(t, item) - min;
        let mut n = r(&mut item.unit_seed, n0) + min;
        if n < 1 {
            n = 1;
        }
        if ovr > 0 {
            n = ovr;
        }
        item.stats.set_base(stat::QUANTITY, 0, n);
    } else {
        let Some(rec) = rec else {
            return Err(Fatal::NoItemRecord);
        };
        if t.is_type(item.record, ty::ARMO as i16) {
            item.stats.set_base(stat::TOBLOCK, 0, i32::from(rec.block));
            item.stats
                .set_base(stat::VELOCITYPERCENT, 0, rec.speed.wrapping_neg());
            durability(item, rec.durability);
            let n = rec.maxac.wrapping_sub(rec.minac).wrapping_add(1) as i32;
            let ac = item.unit_seed.roll_range(rec.minac as i32, n);
            if ac as u32 > rec.maxac {
                return Err(Fatal::Defense);
            }
            item.stats.set_base(stat::ARMORCLASS, 0, ac);
        } else if t.is_type(item.record, ty::WEAP as i16) {
            if rec.stackable != 0 {
                let min = rec.minstack as i32;
                let n0 = total_max_stack(t, item) - min;
                let mut n = r(&mut item.unit_seed, n0) + min;
                if ovr > 0 {
                    n = ovr;
                }
                if n == 0 || item.magic_or_better() {
                    n = 1;
                }
                item.stats.set_base(stat::QUANTITY, 0, n);
            }
            durability(item, rec.durability);
            for (s, v, go) in [
                (stat::MAXDAMAGE, rec.maxdam, true),
                (stat::MINDAMAGE, rec.mindam, true),
                (stat::SECONDARY_MINDAMAGE, rec.mindam2, true),
                (stat::SECONDARY_MAXDAMAGE, rec.maxdam2, true),
                (stat::THROW_MINDAMAGE, rec.minmisdam, rec.maxmisdam != 0),
                (stat::THROW_MAXDAMAGE, rec.maxmisdam, rec.maxmisdam != 0),
            ] {
                if go && v != 0 {
                    item.stats.set_base(s, 0, i32::from(v));
                }
            }
            item.stats
                .set_base(stat::ATTACKRATE, 0, rec.speed.wrapping_neg());
        } else {
            if rec.stackable != 0 {
                let lo = rec.minstack;
                let mut hi = rec.spawnstack;
                if hi < lo || hi == 0 {
                    hi = lo.max(total_max_stack(t, item) as u32);
                }
                let mut n = r(&mut item.unit_seed, hi.wrapping_sub(lo) as i32) + lo as i32;
                if ovr > 0 {
                    n = ovr;
                }
                if item.magic_or_better() || n == 0 {
                    n = 1;
                }
                item.stats.set_base(stat::QUANTITY, 0, n);
            }
            if primary == ty::ELIX as i16 {
                elixir(item);
            }
        }
    }
    if let Some(v) = itype.as_ref().map(|it| it.varinvgfx).filter(|&v| v != 0) {
        item.gfx = item.unit_seed.roll_range(0, i32::from(v));
    }
    let Some(rq) = rq.filter(|_| quest) else {
        return Ok(true);
    };
    let result = dispatch(t, game, item, rq)?;
    if t.item(item.record)
        .is_some_and(|r| r.quest != 0 && r.questdiffcheck != 0)
    {
        let d = i32::from(game.difficulty());
        item.stats
            .list_set(ListKey::ITEM, stat::QUESTITEMDIFFICULTY, 0, d);
        item.flags |= flag::IDENTIFIED;
    }
    Ok(result)
}

/// 3.1.3–3.1.4: durability from the unit seed.
fn durability<S: ItemStats>(item: &mut Item<S>, d: u8) {
    let h = i32::from(d >> 1);
    let v = (r(&mut item.unit_seed, h) + h).min(255);
    item.stats.set_base(stat::DURABILITY, 0, v);
    item.stats.set_base(stat::MAXDURABILITY, 0, i32::from(d));
}

/// The gold setter: a negative value stores 0.
fn set_gold<S: ItemStats>(item: &mut Item<S>, g: i32) {
    item.stats.set_base(stat::GOLD, 0, g.max(0));
}

/// Elixirs, item seed.
fn elixir<S: ItemStats>(item: &mut Item<S>) {
    let k = item.item_seed.roll(ELIXIRS.len() as i32) as usize;
    item.file_index = ELIXIRS[k];
    let v = if matches!(ELIXIRS[k], 9 | 7) {
        ((item.item_seed.step() & 3) as i32 + 1) * 256
    } else {
        1
    };
    item.stats.set_base(stat::VALUE, 0, v);
}

/// "Has durability".
pub fn has_durability<S: ItemStats>(t: &ItemTables, item: &Item<S>) -> bool {
    t.item(item.record)
        .is_some_and(|r| r.nodurability == 0 && r.durability != 0)
        && item.stats.has_stats()
        && item.stats.stat(stat::INDESTRUCTIBLE, 0) < 1
}

/// Max sockets.
pub fn max_sockets<S: ItemStats>(t: &ItemTables, item: &Item<S>) -> i32 {
    max_sockets_at(t, item.record, item.ilvl)
}

/// [`max_sockets`] of the items row `record` at item level `ilvl`.
pub fn max_sockets_at(t: &ItemTables, record: usize, ilvl: i32) -> i32 {
    let (Some(r), Some(it)) = (t.item(record), t.itype_of(record)) else {
        return 0;
    };
    let ilvl = ilvl.max(1);
    let m = if ilvl <= 25 {
        it.maxsock1
    } else if ilvl <= 40 {
        it.maxsock25
    } else {
        it.maxsock40
    };
    i32::from(r.gemsockets.min(m))
}

/// The quality dispatch's normal case: for format ≥
/// 1, for format 0.
pub fn normal_by_format<S: ItemStats>(
    t: &ItemTables,
    game: &dyn ItemGame,
    item: &mut Item<S>,
    rq: &ItemRequest,
) -> Result<(), Fatal> {
    if item.format < 1 {
        normal_legacy(t, game, item, rq)
    } else {
        normal(t, item, rq)
    }
}

/// Normal-quality routine, format 0: one branch by
/// the primary type (no equivalence); any other type gets class skill
/// mods and then the socket roll.
pub fn normal_legacy<S: ItemStats>(
    t: &ItemTables,
    game: &dyn ItemGame,
    item: &mut Item<S>,
    rq: &ItemRequest,
) -> Result<(), Fatal> {
    let Some(primary) = t.item(item.record).map(|r| r.type_) else {
        return Ok(());
    };
    let code = t.item(item.record).map(|r| r.code);
    match u16::try_from(primary).unwrap_or(u16::MAX) {
        ty::PLAY => {
            item.file_index = rq.unit.as_ref().map_or(rq.index, |u| u.class);
            item.flags |= flag::EAR;
        }
        ty::CHAR => super::affixes::charm(t, item, rq)?,
        ty::BOOK => item.suffix[0] = book_row(t, code, false),
        ty::SCRO => item.suffix[0] = book_row(t, code, true),
        ty::BODY => item.file_index = rq.unit.as_ref().map_or(rq.index, |u| u.class),
        _ => {
            class_skill_mods(t, item, rq);
            socket_roll(t, game, item, rq);
        }
    }
    Ok(())
}

/// Normal-quality routine.
pub fn normal<S: ItemStats>(
    t: &ItemTables,
    item: &mut Item<S>,
    rq: &ItemRequest,
) -> Result<(), Fatal> {
    let is = |x: u16| t.is_type(item.record, x as i16);
    let (charm, body, play, scro, book) = (
        is(ty::CHAR),
        is(ty::BODY),
        is(ty::PLAY),
        is(ty::SCRO),
        is(ty::BOOK),
    );
    if charm {
        super::affixes::charm(t, item, rq)?;
    }
    if body || play {
        item.file_index = rq.unit.as_ref().map_or(rq.index, |u| u.class);
        if play {
            item.flags |= flag::EAR;
        }
    }
    let code = t.item(item.record).map(|r| r.code);
    if scro {
        item.suffix[0] = book_row(t, code, true);
    }
    if book {
        item.suffix[0] = book_row(t, code, false);
    }
    class_skill_mods(t, item, rq);
    Ok(())
}

/// The books row whose spell code equals the item code; no
/// row → the books count.
fn book_row(t: &ItemTables, code: Option<[u8; 4]>, scroll: bool) -> u16 {
    let n = t
        .books
        .iter()
        .position(|&(s, b)| Some(if scroll { s } else { b }) == code)
        .unwrap_or(t.books.len());
    n as u16
}

/// One item-seed step as a percentage (`lo′ mod 100`).
fn pct(seed: &mut Seed) -> i32 {
    (seed.step() % 100) as i32
}

/// Class skill mods (staffmods;
/// format 0:).
pub fn class_skill_mods<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, rq: &ItemRequest) {
    let Some(c) = t.itype_of(item.record).map(|it| it.staffmods) else {
        return;
    };
    if c >= 7 {
        return;
    }
    let (count, first) = t.class_skills(usize::from(c));
    let Some(first) = first.filter(|_| count > 0) else {
        return;
    };
    if item.format < 1 {
        class_skill_mods_legacy(item, rq.ilvl, first);
        return;
    }
    let ilvl = rq.ilvl;
    let bonus = if rq.flags2 & req::STAFFMODS_ILVL != 0 {
        rq.ilvl
    } else {
        0
    };
    let v = pct(&mut item.item_seed) + bonus;
    let n = if v > 90 {
        3
    } else if v > 70 {
        2
    } else if v > 30 || bonus != 0 {
        1
    } else {
        return;
    };
    let tier = if ilvl > 36 && item.format >= 100 {
        5
    } else if ilvl > 24 {
        4
    } else if ilvl > 18 {
        3
    } else if ilvl > 11 {
        2
    } else {
        1
    };
    let mut chosen: Vec<u16> = Vec::new();
    for _ in 0..n {
        let p = pct(&mut item.item_seed);
        let mut tr = if p > 80 {
            tier + 1
        } else if p > 30 {
            tier
        } else if p > 10 {
            tier - 1
        } else {
            tier - 2
        };
        if tr < 1 {
            tr = 1;
        }
        if item.quality == q::LOW && tr > 3 {
            tr = 4;
        }
        let mut skill = 0u16;
        for _ in 0..6 {
            let x = item.item_seed.step() % 5;
            skill = first
                .wrapping_add((5 * (tr - 1)) as u16)
                .wrapping_add(x as u16);
            let fits = match t.skills.get(usize::from(skill)) {
                None => true,
                Some(s) => s.itypea1 < 1 || t.is_type(item.record, s.itypea1),
            };
            if fits && !chosen.contains(&skill) {
                chosen.push(skill);
                break;
            }
        }
        // After 6 rejections the last tried skill is used, not listed.
        let value = if item.format < 100 || item.quality != q::LOW {
            let v = pct(&mut item.item_seed) + bonus / 2;
            if v >= 90 {
                3
            } else if v >= 60 {
                2
            } else {
                1
            }
        } else {
            1
        };
        item.stats
            .list_set(ListKey::ITEM, stat::ITEM_SINGLESKILL, skill, value);
    }
}

/// Class skill mods, format 0: no request bonus, no
/// tier 5, no low-quality cap, redraws (without a limit) until the skill
/// is not 73 and not chosen, no `itypea1` test.
fn class_skill_mods_legacy<S: ItemStats>(item: &mut Item<S>, ilvl: i32, first: u16) {
    let p = pct(&mut item.item_seed);
    let n = if p >= 91 {
        3
    } else if p >= 71 {
        2
    } else if p >= 31 {
        1
    } else {
        return;
    };
    let tier = if ilvl >= 25 {
        4
    } else if ilvl >= 19 {
        3
    } else if ilvl >= 12 {
        2
    } else {
        1
    };
    let mut chosen: Vec<u16> = Vec::new();
    for _ in 0..n {
        let p = pct(&mut item.item_seed);
        let tr = if p >= 81 {
            tier + 1
        } else if p >= 31 {
            tier
        } else if p >= 11 {
            tier - 1
        } else {
            tier - 2
        }
        .max(1);
        // At most 3 skills are chosen and one is 73, so one of the five
        // candidates is always free: the loop ends.
        let skill = loop {
            let x = item.item_seed.step() % 5;
            let skill = first
                .wrapping_add((5 * (tr - 1)) as u16)
                .wrapping_add(x as u16);
            if skill != 73 && !chosen.contains(&skill) {
                break skill;
            }
        };
        chosen.push(skill);
        let v = pct(&mut item.item_seed);
        let value = if v >= 90 {
            3
        } else if v >= 60 {
            2
        } else {
            1
        };
        item.stats
            .list_set(ListKey::ITEM, stat::ITEM_SINGLESKILL, skill, value);
    }
}

/// Generation socket roll.
pub fn socket_roll<S: ItemStats>(
    t: &ItemTables,
    game: &dyn ItemGame,
    item: &mut Item<S>,
    rq: &ItemRequest,
) {
    let Some(rec) = t.item(item.record) else {
        return;
    };
    let mut m = max_sockets(t, item);
    if item.quality < q::NORMAL || rec.hasinv == 0 || rec.stackable != 0 || m == 0 {
        return;
    }
    let cap = match game.difficulty() {
        0 => 3,
        1 => 4,
        _ => 6,
    };
    m = m.min(cap);
    if item.format < 100 && t.is_type(item.record, ty::TORS as i16) {
        return;
    }
    let mut p = item.item_seed.roll(100);
    if rq.flags2 & req::NO_SOCKETS != 0 {
        return;
    }
    if rq.flags2 & req::ALWAYS_SOCKETS != 0 {
        p = 0;
    }
    if rq.flags1 & flag::SOCKETED != 0 || p < 33 {
        item.flags |= flag::SOCKETED;
        let n = (item.start_seed % m as u32) as i32 + 1;
        socket_count(t, item, n);
    }
}

/// Socket count.
pub fn socket_count<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, n: i32) {
    let Some(rec) = t.item(item.record) else {
        return;
    };
    let mut n = n;
    let mut cap = (i32::from(rec.invwidth) * i32::from(rec.invheight)).min(6);
    if cap == 0 {
        return;
    }
    match item.quality {
        q::MAGIC => cap = cap.min(4),
        q::RARE => cap = cap.min(2),
        q::CRAFTED | q::TEMPERED => cap = cap.min(3),
        q::SET | q::UNIQUE => {
            let cur = item.stats.stat(stat::NUMSOCKETS, 0);
            if cur < 1 {
                cap = 1;
            } else {
                n = cur;
            }
        }
        _ => {}
    }
    cap = cap.min(max_sockets(t, item));
    if cap < 1 {
        return;
    }
    let result = n.max(1).min(cap);
    item.flags |= flag::SOCKETED;
    item.stats.set_base(stat::NUMSOCKETS, 0, result);
}

/// Ethereal roll.
pub fn ethereal_roll<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, rq: &ItemRequest) {
    let quest = t.item(item.record).is_some_and(|r| r.quest != 0);
    let wa = t.is_type(item.record, ty::WEAP as i16) || t.is_type(item.record, ty::ARMO as i16);
    if rq.flags2 & req::NEVER_ETHEREAL != 0
        || !wa
        || !has_durability(t, item)
        || item.quality == q::LOW
        || item.quality == q::SET
        || quest
    {
        return;
    }
    let p = item.item_seed.roll(100);
    if rq.flags2 & req::ALWAYS_ETHEREAL != 0 || rq.flags1 & flag::ETHEREAL != 0 || p < 5 {
        apply_ethereal(t, item);
        if has_durability(t, item) {
            let m = item.stats.base(stat::MAXDURABILITY, 0) / 2 + 1;
            item.stats.set_base(stat::MAXDURABILITY, 0, m);
            item.stats.set_base(stat::DURABILITY, 0, m);
        }
    }
}

/// Apply ethereal (D2MOO `ITEMMODS_ApplyEthereality`).
pub fn apply_ethereal<S: ItemStats>(t: &ItemTables, item: &mut Item<S>) {
    item.flags |= flag::ETHEREAL;
    let stats: &[u16] = if t.is_type(item.record, ty::WEAP as i16) {
        &[
            stat::MINDAMAGE,
            stat::MAXDAMAGE,
            stat::SECONDARY_MINDAMAGE,
            stat::SECONDARY_MAXDAMAGE,
            stat::THROW_MINDAMAGE,
            stat::THROW_MAXDAMAGE,
        ]
    } else {
        &[stat::ARMORCLASS]
    };
    for &s in stats {
        let v = item.stats.base(s, 0).wrapping_mul(3) / 2;
        item.stats.set_base(s, 0, v);
    }
}

/// (forced requests).
fn forced<S: ItemStats>(t: &ItemTables, item: &mut Item<S>, rq: &mut ItemRequest) {
    let copy = |item: &mut Item<S>, bits: u32| item.flags = item.flags & !bits | rq.flags1 & bits;
    copy(item, flag::IDENTIFIED | flag::NOSELL);
    // Step 2, format 0 only: the forced socket count (no draw). The three
    // flag copies that follow run for every forced request (OQ-G2).
    let primary = t.item(item.record).map(|r| r.type_);
    if item.format == 0
        && primary != Some(ty::TORS as i16)
        && rq.flags1 & flag::SOCKETED != 0
        && item.flags & flag::SOCKETED == 0
    {
        match max_sockets(t, item) {
            0 => item.flags &= !flag::SOCKETED,
            m => socket_count(t, item, (item.start_seed % m as u32) as i32 + 1),
        }
    }
    copy(item, flag::SOCKETED | flag::BROKEN | flag::STARTITEM);
    if t.item(item.record)
        .is_some_and(|r| r.type_ == ty::GOLD as i16)
    {
        set_gold(item, rq.quantity);
    } else {
        item.stats.set_base(stat::QUANTITY, 0, rq.quantity);
    }
    let max = (rq.max_dur as u32).min(255);
    let min = (rq.min_dur as u32).min(max);
    rq.max_dur = max as i32;
    rq.min_dur = min as i32;
    item.stats.set_base(stat::DURABILITY, 0, min as i32);
    item.stats.set_base(stat::MAXDURABILITY, 0, max as i32);
}

fn player(rq: &ItemRequest) -> Option<&PlayerInfo> {
    rq.unit.as_ref().and_then(|u| u.player.as_ref())
}

/// Step 5, ears (primary type `play`).
fn ear<S>(item: &mut Item<S>, rq: &ItemRequest) -> Result<(), CreateError> {
    if rq.force {
        item.name = rq.name;
        item.ear_level = rq.ear_level;
        item.flags = item.flags & !flag::NAMED | rq.flags1 & flag::NAMED;
        return Ok(());
    }
    let p = player(rq).ok_or(CreateError::NotPlayer)?;
    item.name = p.name;
    item.ear_level = p.level;
    if let Some(hc) = p.hardcore {
        item.flags = item.flags & !flag::NAMED | if hc { flag::NAMED } else { 0 };
    }
    Ok(())
}

/// Step 5, personalized names.
pub(crate) fn personalize<S>(item: &mut Item<S>, rq: &ItemRequest) -> Result<(), CreateError> {
    item.name = if rq.force {
        rq.name
    } else {
        player(rq).ok_or(Fatal::NoPlayerData)?.name
    };
    Ok(())
}

/// Replenish timers: the frame to
/// schedule event 3 at, for stat 252 then 253, when `scheduled` (an
/// event 3 already on the item) is false.
pub fn replenish_timer<S: ItemStats>(stats: &S, scheduled: bool, frame: u32) -> Option<u32> {
    replenish_timer_at(stats, scheduled, frame, crate::tick::TickRate::BASE)
}

/// [`replenish_timer`] at the game's tick rate: the 2500 / rate delay is a
/// count of base ticks.
pub fn replenish_timer_at<S: ItemStats>(
    stats: &S,
    scheduled: bool,
    frame: u32,
    rate: crate::tick::TickRate,
) -> Option<u32> {
    if scheduled {
        return None;
    }
    [stat::REPLENISH_DURABILITY, stat::REPLENISH_QUANTITY]
        .into_iter()
        .map(|s| stats.stat(s, 0))
        .find(|&v| v != 0)
        .map(|v| frame.wrapping_add(rate.ticks(2500 / v + 1) as u32))
}
