//! The TC walk, NoDrop scaling, creation inputs
//! and placement and the gold rules.

use super::quality::roll_quality;
use super::runtime::{
    find_item_code, item_is_type, TreasureClass, FLAG_NOT_CLASSIC, FLAG_SET, FLAG_TC, FLAG_UNIQUE,
};
use super::softfloat::F64;
use super::{TreasureData, TreasureError};
use crate::rng::Seed;

/// Most slots on the walk's stack.
pub const MAX_SLOTS: usize = 64;
/// Default `max`.
pub const DEFAULT_MAX: i32 = 6;
/// Classic throwables re-picked before turning into `lsd`.
pub const THROWABLE_LIMIT: i32 = 10;
/// itemtypes record 4 `gold`.
pub const TYPE_GOLD: usize = 4;
/// monstats record 391 `hellbovine`.
pub const CLASS_HELLBOVINE: u32 = 391;
/// Spawn type and init flags of a drop request.
pub const SPAWN_TYPE_DROP: u8 = 3;
pub const INIT_FLAGS_DROP: u32 = 1;

/// Game state the walk reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameFacts {
    /// game +0x70.
    pub expansion: bool,
    /// game +0x6D, 0–2.
    pub difficulty: u8,
    /// game +0x6A (3 = single player).
    pub game_type: u8,
    /// Living players in the game (before the `players`
    /// setting; "living" = not dead).
    pub living_players: i32,
    /// The `players` setting `S`, 0–8.
    pub players_setting: i32,
    /// game +0x78, passed through to the drop request.
    pub item_format: u32,
}

/// What the dropping unit `U` is, for the player factor (step 4) and the drop (step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropperKind {
    /// No unit (item level 1).
    None,
    /// A monster: its class, `level` stat (12) and
    /// `monster_playercount` stat (100).
    Monster {
        class: u32,
        level: i32,
        playercount: i32,
    },
    /// A player: its base `level`.
    Player { level: i32 },
    /// Anything else (objects): the area level of its level (`a`,
    /// [`super::area_level`]).
    Other { area_level: i32 },
}

/// The dropping unit `U` (its seed is passed separately).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dropper {
    pub kind: DropperKind,
    pub x: i32,
    pub y: i32,
}

/// The recipient `R` (killer or operator), when it exists. Fields are
/// staged by the caller from unit and stat state (seams: units/stats).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recipient {
    /// With `O` = `R` if `R` is a player, else `R`'s minion
    /// owner: `Some(k)` when `O` is a player, `k` = living members of
    /// `O`'s party in `O`'s level (1 when alone); `None`
    /// otherwise.
    pub party: Option<i32>,
    /// `M`: stat 80 of `R` plus its minion owner's when `R` is
    /// a player or monster, else 0.
    pub magic_find: i32,
    /// Stat 79 of `R` plus its minion owner's.
    pub gold_find: i32,
}

/// One item creation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropRequest<S> {
    pub id: u16,
    pub quality: u8,
    /// uniqueitems / setitems record + 1, or 0.
    pub index: i32,
    pub item_level: i32,
    /// Room and position from [`DropSink::place`].
    pub spot: S,
    pub spawn_type: u8,
    pub init_flags: u32,
    pub item_format: u32,
    /// `d` (0x04, 0x10) plus 0x01 for a `hellbovine` dropper.
    pub drop_flags: u8,
}

/// The seam to item creation and placement. Providers: the items
/// session (`d2_sim::items`: creation, gold stat 14) and
/// the collision (free-spot search).
pub trait DropSink {
    /// A room and position.
    type Spot;
    /// A created item unit.
    type Item: Copy;
    /// for a dropper at (`x`, `y`): start at (`x` + 2, `y` + 3)
    /// when a room exists there, else (`x`, `y`); the free-spot search
    ///`(room, start, (`x`, `y`), 1, 0x3E01, 0x801, 1). Items
    /// of one walk are placed one after another, each seeing the
    /// previous ones. `None`: no spot.
    fn place(&mut self, x: i32, y: i32) -> Option<Self::Spot>;
    /// Creates the item; `None` on failure. The gold base
    /// ([`gold_base`]) happens in here.
    fn create(&mut self, req: DropRequest<Self::Spot>) -> Option<Self::Item>;
    /// The item's gold amount (stat 14).
    fn gold(&self, item: Self::Item) -> i32;
    /// Stores a gold amount as given; callers clamp negatives.
    fn set_gold(&mut self, item: Self::Item, value: i32);
}

/// Walk arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkArgs {
    /// TC index; `None` is fatal (0xF3A).
    pub tc: Option<u16>,
    /// Forced quality `Q` (0 = roll).
    pub quality: u8,
    /// `L`.
    pub level: i32,
    /// `F` = 1 (Find Item): NoDrop off.
    pub find_item: bool,
    /// Whether the caller passes an output list.
    pub list: bool,
    pub max: i32,
}

/// The player factor `n`, steps 1–4 (no draws).
pub fn player_factor(game: &GameFacts, dropper: &Dropper, recipient: Option<&Recipient>) -> i32 {
    let p = match recipient.and_then(|r| r.party) {
        Some(k) if k >= 2 => k.min(8),
        _ => 1,
    };
    let mut players = game.living_players;
    if matches!(game.game_type, 1..=3) {
        players = players.max(game.players_setting);
    }
    let mut n = p.wrapping_add(players.wrapping_sub(p) / 2);
    if let DropperKind::Monster { playercount, .. } = dropper.kind {
        n = n.min(playercount.max(1));
    }
    n
}

/// NoDrop `N` for nodrop `n0` ≠ 0, TC total `c` and
/// player factor `n`, in binary64 with round to nearest even.
///
/// TODO(treasure): x87 precision control is unconfirmed; values
/// outside the binary64 normal range or i32 are reported as
/// [`TreasureError::NoDropRange`] (none occur on the base game data).
pub fn nodrop(n0: i32, c: i32, n: i32) -> Result<i32, TreasureError> {
    if n <= 1 {
        return Ok(n0);
    }
    let fl = |v: i64| F64::from_i64(v).map_err(|_| TreasureError::NoDropRange);
    let ck = |v: Result<F64, _>| v.map_err(|_| TreasureError::NoDropRange);
    let cf = fl(i64::from(c))?;
    let x = ck(fl(i64::from(n0))?.div_rne(fl(i64::from(n0) + i64::from(c))?))?;
    let mut p = x;
    for _ in 1..n {
        p = ck(p.mul_rne(x))?;
    }
    let q = ck(F64::ONE.sub_rne(p))?;
    if q.is_zero() {
        return Ok(0);
    }
    let one_minus_q = ck(F64::ONE.sub_rne(q))?;
    let v = ck(ck(one_minus_q.mul_rne(cf))?.div_rne(q))?;
    v.trunc_i32().map_err(|_| TreasureError::NoDropRange)
}

/// Entry selection. `None`: no entry.
pub fn select_entry(tc: &TreasureClass, r: i32, expansion: bool) -> Option<usize> {
    let e = &tc.entries;
    if expansion {
        let (mut lo, mut hi) = (0usize, e.len());
        while lo < hi {
            let m = lo + (hi - lo) / 2;
            let s = e[m].start_expansion;
            if s < r {
                lo = m + 1;
            } else if s > r {
                hi = m;
            } else {
                return Some(m);
            }
        }
        // An empty TC never gets here (its slot ends on total 0); D2RUST
        // selects none rather than index 0 of nothing.
        (!e.is_empty()).then(|| lo.saturating_sub(1))
    } else {
        (0..e.len()).find(|&i| {
            e[i].flags & FLAG_NOT_CLASSIC == 0 && (i + 1 == e.len() || e[i + 1].start_classic > r)
        })
    }
}

/// Item level.
pub fn item_level(dropper: &Dropper) -> i32 {
    let l = match dropper.kind {
        DropperKind::None => 1,
        DropperKind::Monster { level, .. } => level,
        DropperKind::Player { level } => level,
        DropperKind::Other { area_level } => area_level,
    };
    l.max(1)
}

/// Gold base (inside item creation, for the
/// items provider): `roll(5 × ilvl)` on the new item's seed + ilvl, at
/// least 1; a drop-request quantity > 0 replaces it.
pub fn gold_base(item_seed: &mut Seed, ilvl: i32, quantity: i32) -> i32 {
    let mut g = (item_seed.roll(ilvl.wrapping_mul(5)) as i32).wrapping_add(ilvl);
    if g <= 0 {
        g = 1;
    }
    if quantity > 0 {
        g = quantity;
    }
    g
}

/// One slot of the walk's stack.
#[derive(Debug, Clone, Copy)]
struct Slot {
    tc: Option<u16>,
    left: i32,
    mods: [u16; 6],
}

fn max_mods(a: [u16; 6], b: &[u16; 6]) -> [u16; 6] {
    std::array::from_fn(|j| a[j].max(b[j]))
}

/// The TC walk. Draws on `seed` (U's seed) in the order of
/// the treasure randomness. Returns the created items in order (at most `max`); a
/// caller without an output list ignores them.
pub fn walk<S: DropSink>(
    data: &TreasureData,
    game: &GameFacts,
    dropper: &Dropper,
    seed: &mut Seed,
    recipient: Option<&Recipient>,
    args: &WalkArgs,
    sink: &mut S,
) -> Result<Vec<S::Item>, TreasureError> {
    let tcs = &data.tcs.tcs;
    let tc0 = args
        .tc
        .filter(|&t| usize::from(t) < tcs.len())
        .ok_or(TreasureError::NoTc)?;
    let max = if !args.list {
        if args.max < 1 {
            DEFAULT_MAX
        } else {
            args.max
        }
    } else if args.max == 0 {
        return Err(TreasureError::ZeroMax);
    } else {
        args.max
    };
    let exp = game.expansion;
    let n = player_factor(game, dropper, recipient);
    let mf = recipient.map_or(0, |r| r.magic_find);
    let first = &tcs[usize::from(tc0)];
    let mut slots = vec![Slot {
        tc: Some(tc0),
        left: first.picks.wrapping_abs().max(1),
        mods: first.mods,
    }];
    let mut k = 0usize;
    let mut throwables = 0i32;
    let mut out = Vec::new();

    'walk: loop {
        loop {
            let slot = &mut slots[k];
            let Some(ti) = slot.tc else { break };
            let tc = &tcs[usize::from(ti)];
            let total = tc.total(exp);
            if total == 0 || slot.left == 0 {
                break;
            }
            let r = if tc.picks < 0 {
                let r = tc.picks.wrapping_abs().wrapping_sub(slot.left);
                if r >= total {
                    break;
                }
                r
            } else if args.find_item {
                seed.roll(total) as i32
            } else {
                let nd = if tc.nodrop == 0 {
                    0
                } else {
                    nodrop(tc.nodrop, total, n)?
                };
                let v = seed.roll(total.wrapping_add(nd)) as i32;
                if v < nd {
                    slot.left -= 1;
                    continue;
                }
                v - nd
            };
            slot.left -= 1;
            let Some(ei) = select_entry(tc, r, exp) else {
                continue;
            };
            let e = &tc.entries[ei];

            if e.flags & FLAG_TC != 0 {
                let x = data.tcs.get(e.id, 0);
                let (left, mods) = match x {
                    Some(xi) => {
                        let xt = &tcs[usize::from(xi)];
                        (
                            xt.picks.wrapping_abs().max(1),
                            max_mods(slot.mods, &xt.mods),
                        )
                    }
                    // TODO(treasure): with `get` giving none leaves the
                    // new slot's picks and mods unstated; the slot ends at
                    // once, so they are never read.
                    None => (1, slot.mods),
                };
                let new = Slot { tc: x, left, mods };
                if slot.left > 0 {
                    k += 1;
                    if k >= MAX_SLOTS {
                        return Err(TreasureError::SlotOverflow);
                    }
                    slots.truncate(k);
                    slots.push(new);
                } else {
                    *slot = new;
                }
                continue;
            }

            // item entry.
            if e.id == 0xFFFF {
                continue;
            }
            let mut id = i32::from(e.id);
            let item = data.items.get(usize::from(e.id));
            if !exp {
                if item.is_none_or(|i| i.version >= 100) {
                    continue;
                }
                slot.mods = max_mods(slot.mods, &e.mods);
            }
            let (quality, index) = if e.flags & FLAG_UNIQUE != 0 {
                (7, i32::from(e.row) + 1)
            } else if e.flags & FLAG_SET != 0 {
                (5, i32::from(e.row) + 1)
            } else if args.quality != 0 {
                (args.quality, 0)
            } else {
                (
                    roll_quality(data, e.id, args.level, mf, &slot.mods, seed)?,
                    0,
                )
            };
            let mut d = 0u8;
            if slot.mods[4] != 0 && (seed.step() & 0x3FF) < u32::from(slot.mods[4]) {
                d = 0x04;
            }
            if slot.mods[5] != 0 && (seed.step() & 0x3FF) < u32::from(slot.mods[5]) {
                d |= 0x10;
            }
            let throwable = item
                .and_then(|i| data.itemtypes.get(usize::from(i.type_)))
                .is_some_and(|t| t.throwable != 0);
            if !exp && throwable {
                let read = throwables;
                throwables += 1;
                if read < THROWABLE_LIMIT {
                    slot.left += 1;
                    continue;
                }
                id = find_item_code(data.items, *b"lsd ").map_or(-1, |i| i as i32);
            }
            let row = e.row;
            let Some(it) = create(game, dropper, sink, id, quality, index, d) else {
                continue;
            };
            let created = data.items.get(id as usize);
            // TODO(treasure, gold equivalents): "of type 4 or an equivalent"
            // is read as the item type test
            // (type or nonzero type2); whether type2 counts is unconfirmed.
            if row != 0 && created.is_some_and(|i| item_is_type(data.equiv, i, TYPE_GOLD)) {
                let g = sink.gold(it).wrapping_mul(i32::from(row)) >> 8;
                sink.set_gold(it, g.max(0));
            }
            out.push(it);
            if out.len() as i32 >= max {
                break 'walk;
            }
            // gold find.
            if let Some(r) = recipient {
                if created.is_some_and(|i| usize::from(i.type_) == TYPE_GOLD) {
                    let g = sink.gold(it).wrapping_mul(100i32.wrapping_add(r.gold_find)) / 100;
                    sink.set_gold(it, g.max(0));
                }
            }
        }
        if k == 0 {
            break;
        }
        k -= 1;
    }
    Ok(out)
}

/// Placement and the drop request.
#[allow(clippy::too_many_arguments)]
fn create<S: DropSink>(
    game: &GameFacts,
    dropper: &Dropper,
    sink: &mut S,
    id: i32,
    quality: u8,
    index: i32,
    d: u8,
) -> Option<S::Item> {
    if id < 0 {
        return None;
    }
    let spot = sink.place(dropper.x, dropper.y)?;
    let bovine =
        matches!(dropper.kind, DropperKind::Monster { class, .. } if class == CLASS_HELLBOVINE);
    sink.create(DropRequest {
        id: id as u16,
        quality,
        index,
        item_level: item_level(dropper),
        spot,
        spawn_type: SPAWN_TYPE_DROP,
        init_flags: INIT_FLAGS_DROP,
        item_format: game.item_format,
        drop_flags: d | u8::from(bovine),
    })
}
