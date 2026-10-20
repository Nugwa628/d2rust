//! The quest drop helper: item level, item class (the
//! drop code, else the random class pick with the magic
//! retry loop) and the class sub-pickers (armor),
//! (weapons), (misc). Placement and
//! creation go through the walk's [`DropSink`], as the walk's do. The pickers are
//! [`super::class_pick`]'s (one implementation, shared with the object
//! drop helpers); the action wiring's drop is
//! `wiring::economy::drop_helpers::source_drop`.

use super::class_pick::{self, ClassPicks, PickError};
use super::walk::{item_level, DropRequest, DropSink, Dropper, INIT_FLAGS_DROP, SPAWN_TYPE_DROP};
use super::TreasureError;
use crate::items::tables::{ItemRec, ItemTables};
use crate::rng::Seed;

/// Quality 4 (magic): the step 3 retry loop.
pub const QUALITY_MAGIC: u8 = 4;
/// Highest item level of the class pick (fatal 0x180 above).
pub const MAX_PICK_LEVEL: i32 = class_pick::MAX_LEVEL;
/// Calls of the random class pick in the magic loop before the weapons picker
/// takes over.
pub const MAGIC_RETRIES: u32 = class_pick::MAGIC_RANDOM_REPICKS;
/// Candidates held by a sub-picker.
pub const MAX_CANDIDATES: usize = class_pick::MAX_CANDIDATES;
/// itemtypes 40 `body part`.
pub const TYPE_BODY_PART: i16 = class_pick::TYPE_BODY;
/// The `gld ` code (cached index `0x008846EC`).
pub const GOLD_CODE: [u8; 4] = class_pick::GOLD_CODE;
/// D2RUST guard for the unbounded magic loop of step 3: the base game never
/// leaves it when no weapon can be magic (the server hangs); D2RUST stops
/// after this many retries with [`TreasureError::QuestDropHang`].
pub const MAGIC_LOOP_GUARD: u32 = class_pick::MAGIC_LOOP_GUARD;

/// A part of the combined items array.
pub use super::class_pick::Part;

/// The tables and game facts the class pick reads.
#[derive(Clone, Copy)]
pub struct PickData<'a> {
    pub items: &'a [ItemRec],
    /// (start, count) per part ([`ItemTables::parts`]).
    pub parts: [Option<(usize, usize)>; 3],
    /// game +0x70.
    pub expansion: bool,
}

impl<'a> PickData<'a> {
    pub fn of(t: &'a ItemTables, expansion: bool) -> Self {
        Self {
            items: &t.items,
            parts: t.parts,
            expansion,
        }
    }

    /// The pick rows of [`class_pick`], the one implementation of the
    /// sub-pickers, the random class and the magic loop (shared with
    ///–).
    pub fn picks(&self) -> ClassPicks {
        ClassPicks::of_items(self.items, self.parts)
    }
}

/// The fatal asserts of the class pick as treasure errors.
fn pick_error(e: PickError) -> TreasureError {
    match e {
        PickError::Level(l) => TreasureError::PickLevel(l),
        PickError::NoGold => TreasureError::NoGold,
        PickError::Code(c) => TreasureError::DropCode(c.to_le_bytes()),
        PickError::Hang => TreasureError::QuestDropHang,
    }
}

/// A class sub-picker: the combined index, or −1 (part absent,
/// or no candidate: D2RUST's choice for the uninitialised slot).
/// [`class_pick::part_pick`].
#[allow(clippy::too_many_arguments)]
pub fn sub_pick(
    d: &PickData<'_>,
    part: Part,
    seed: &mut Seed,
    l: i32,
    p6: i32,
    p7: i32,
    monster: bool,
) -> i32 {
    class_pick::part_pick(&d.picks(), part, seed, l, p6, p7 != 0, monster, d.expansion)
}

/// The random class pick ([`class_pick::random_class`]): a missing
/// `gld ` row is fatal 0x17C.
pub fn class_pick(
    d: &PickData<'_>,
    seed: &mut Seed,
    l: i32,
    p6: i32,
    p7: i32,
    monster: bool,
) -> Result<i32, TreasureError> {
    class_pick::random_class(&d.picks(), seed, l, p6, p7 != 0, monster, d.expansion)
        .map_err(pick_error)
}

/// The item class `c` of step 3 ([`class_pick::source_class`]).
/// `drop_code`: the unit's drop item code (unit +0xB8), `None` when 0.
#[allow(clippy::too_many_arguments)]
pub fn quest_class(
    d: &PickData<'_>,
    drop_code: Option<[u8; 4]>,
    q: u8,
    seed: &mut Seed,
    l: i32,
    p6: i32,
    p7: i32,
    monster: bool,
) -> Result<i32, TreasureError> {
    let code = drop_code.map_or(0, u32::from_le_bytes);
    class_pick::source_class(
        &d.picks(),
        seed,
        code,
        l,
        q,
        p6,
        p7 != 0,
        monster,
        d.expansion,
    )
    .map_err(pick_error)
}

/// The arguments of the quest drop besides the game and the source unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuestDropArgs {
    /// Quality `q`.
    pub quality: u8,
    /// The unit's drop item code (+0xB8), `None` when 0.
    pub drop_code: Option<[u8; 4]>,
    /// Item type filter (−1 = any).
    pub p6: i32,
    /// Skip the rarity roll when ≠ 0.
    pub p7: i32,
    /// The game's item format (game +0x78).
    pub item_format: u32,
}

/// What the helper writes out: `*&level` (always, step 2) and the request
/// (only when a spot was found, step 4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestDropOut<S, I> {
    pub level: i32,
    pub request: Option<DropRequest<S>>,
    pub item: Option<I>,
}

/// for the dropper `u` (its kind, level and
/// position), drawing on `seed` (the unit seed of `U`; `None` for no
/// unit). The pipeline's ilvl write-back is the sink's.
pub fn quest_drop<S: DropSink>(
    d: &PickData<'_>,
    u: &Dropper,
    monster: bool,
    seed: &mut Seed,
    args: &QuestDropArgs,
    sink: &mut S,
) -> Result<QuestDropOut<S::Spot, S::Item>, TreasureError>
where
    S::Spot: Clone,
{
    // Step 2.
    let level = item_level(u);
    // Step 3.
    let c = quest_class(
        d,
        args.drop_code,
        args.quality,
        seed,
        level,
        args.p6,
        args.p7,
        monster,
    )?;
    // Step 4.
    let Some(spot) = sink.place(u.x, u.y) else {
        return Ok(QuestDropOut {
            level,
            request: None,
            item: None,
        });
    };
    // Step 5. A class of −1 fails creation.
    let req = DropRequest {
        id: c as u16,
        quality: args.quality,
        index: 0,
        item_level: level,
        spot,
        spawn_type: SPAWN_TYPE_DROP,
        init_flags: INIT_FLAGS_DROP,
        item_format: args.item_format,
        drop_flags: 0,
    };
    let item = if c < 0 {
        None
    } else {
        sink.create(req.clone())
    };
    Ok(QuestDropOut {
        level,
        request: Some(req),
        item,
    })
}

#[cfg(test)]
mod tests;
