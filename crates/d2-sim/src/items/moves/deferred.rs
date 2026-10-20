//! Deferred item messages: marking, the per-client dispatcher
//! Driven by `items/item-actions.tsv`, ground items
//! and the direct sends. Message bytes per the layouts
//! ([`super::layouts`]).

use super::layouts;
use super::seams::MoveWorld;
use super::{iflag, mode, uflag, Guid, MoveFatal, Owner};

/// Which flags a row tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Test {
    /// Command flags (item data +0x14).
    Cmd,
    /// Item flags (item data +0x18).
    Item,
}

/// Recipients of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum To {
    /// Only the owner's client.
    Owner,
    /// Every client that processes the player.
    All,
    /// The owner, or any client when the item is in mode 1.
    OwnerOrMode1,
}

/// Extra condition of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    None,
    /// Item flag 0x100 clear.
    NotBroken,
    /// Mode in 0..2; neither 0x100 nor 0x200; the owner's client gets
    /// nothing while the item has item flag 0x40000.
    UpdateStats,
}

/// One row of `items/item-actions.tsv`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemAction {
    pub order: u8,
    pub test: Test,
    pub flags: u32,
    pub to: To,
    pub cond: Cond,
    pub message: u8,
    /// 0x9C / 0x9D action byte, or the 0x7D flag value.
    pub action: u32,
    pub sender: u32,
    pub d2moo: &'static str,
}

#[allow(clippy::too_many_arguments)]
const fn row(
    order: u8,
    test: Test,
    flags: u32,
    to: To,
    cond: Cond,
    message: u8,
    action: u32,
    sender: u32,
    d2moo: &'static str,
) -> ItemAction {
    ItemAction {
        order,
        test,
        flags,
        to,
        cond,
        message,
        action,
        sender,
        d2moo,
    }
}

use Cond as C;
use Test as T;
use To as O;

/// The dispatcher's rows in test order (`items/item-actions.tsv`).
pub const ITEM_ACTIONS: [ItemAction; 20] = [
    row(
        1,
        T::Cmd,
        0x40,
        O::Owner,
        C::None,
        0x9C,
        0x01,
        0x0053EC20,
        "GroundToCursor",
    ),
    row(
        2,
        T::Cmd,
        0x80000,
        O::Owner,
        C::None,
        0x9D,
        0x16,
        0x0053D4B0,
        "Unknown0x16",
    ),
    row(
        3,
        T::Cmd,
        0x2 | 0x80,
        O::Owner,
        C::None,
        0x9C,
        0x04,
        0x0053ED50,
        "PutInContainer",
    ),
    row(
        4,
        T::Cmd,
        0x4,
        O::Owner,
        C::None,
        0x9D,
        0x05,
        0x0053D010,
        "RemoveFromContainer",
    ),
    row(
        5,
        T::Cmd,
        0x8 | 0x200,
        O::All,
        C::None,
        0x9D,
        0x06,
        0x0053D090,
        "Equip",
    ),
    row(
        6,
        T::Cmd,
        0x10000,
        O::All,
        C::None,
        0x9D,
        0x07,
        0x0053D0B0,
        "IndirectlySwapBodyItem",
    ),
    row(
        7,
        T::Cmd,
        0x10,
        O::All,
        C::None,
        0x9D,
        0x08,
        0x0053D0D0,
        "Unequip",
    ),
    row(
        8,
        T::Cmd,
        0x20,
        O::All,
        C::None,
        0x9D,
        0x09,
        0x0053D0F0,
        "SwapBodyItem",
    ),
    row(
        9,
        T::Cmd,
        0x100,
        O::Owner,
        C::None,
        0x9C,
        0x0A,
        0x0053EDB0,
        "AddQuantity",
    ),
    row(
        10,
        T::Cmd,
        0x40000,
        O::Owner,
        C::None,
        0x9C,
        0x0D,
        0x0053EE10,
        "SwapInContainer",
    ),
    row(
        11,
        T::Cmd,
        0x400 | 0x2000,
        O::Owner,
        C::None,
        0x9C,
        0x0E,
        0x0053EE70,
        "PutInBelt",
    ),
    row(
        12,
        T::Cmd,
        0x800,
        O::Owner,
        C::None,
        0x9C,
        0x0F,
        0x0053EED0,
        "RemoveFromBelt",
    ),
    row(
        13,
        T::Cmd,
        0x1000,
        O::Owner,
        C::None,
        0x9C,
        0x10,
        0x0053EEF0,
        "SwapInBelt",
    ),
    row(
        14,
        T::Cmd,
        0x100000,
        O::Owner,
        C::None,
        0x9C,
        0x12,
        0x0053EF10,
        "ToCursor",
    ),
    row(
        15,
        T::Cmd,
        0x200000,
        O::All,
        C::None,
        0x9D,
        0x17,
        0x0053D110,
        "WeaponSwitch",
    ),
    row(
        16,
        T::Cmd,
        0x4000,
        O::All,
        C::None,
        0x9D,
        0x11,
        0x0053D2C0,
        "AutoUnequip",
    ),
    row(
        17,
        T::Cmd,
        0x8000,
        O::All,
        C::None,
        0x9D,
        0x14,
        0x0053D350,
        "Unknown0x14",
    ),
    row(
        18,
        T::Item,
        0x100,
        O::OwnerOrMode1,
        C::None,
        0x7D,
        0x100,
        0x0053D440,
        "broken",
    ),
    row(
        19,
        T::Item,
        0x200,
        O::OwnerOrMode1,
        C::NotBroken,
        0x7D,
        0x200,
        0x0053D440,
        "repaired",
    ),
    row(
        20,
        T::Item,
        0x1,
        O::OwnerOrMode1,
        C::UpdateStats,
        0x9D,
        0x15,
        0x0053D420,
        "UpdateStats",
    ),
];

/// Owner refresh: queue the unit for
/// update, set unit +0xC8 bit 0 and, for players, bit 1.
pub fn owner_refresh<W: MoveWorld>(w: &mut W, u: Owner) {
    w.queue_update(u);
    let bits = if u.is_player() { 0x3 } else { 0x1 };
    let v = w.update_bits(u);
    w.set_update_bits(u, v | bits);
}

/// Marking: command flags, update list, owner refresh.
pub fn mark<W: MoveWorld>(w: &mut W, owner: Owner, item: Guid, cmd: u32) {
    let v = w.cmd_flags(item);
    w.set_cmd_flags(item, v | cmd);
    w.update_list_add(owner, item);
    owner_refresh(w, owner);
}

/// Category byte.
pub fn category<W: MoveWorld>(w: &W, item: Guid) -> u8 {
    let c = w.component(item);
    let loc = w.body_loc(item);
    if loc != 4 && loc != 5 {
        return c;
    }
    let Some(o) = w.item_owner(item) else {
        return c;
    };
    // on each hand: item flag 0x100 or 0x4000 fails it
    // (measured: two
    // Throwing Axes flagged 0x4000 at the join give category 5).
    let hand = |l| {
        w.body_item(o, l).filter(|&h| {
            w.item_flags(h) & (iflag::BROKEN | iflag::NOEQUIP) == 0 && !w.two_handed(h)
        })
    };
    let class = w.unit_class(o);
    let dual = match o.ty {
        Owner::PLAYER => class == 4 || class == 6,
        Owner::MONSTER => class == 0x1A1 || class == 0x1A2,
        _ => false,
    };
    if hand(4).is_some() && hand(5).is_some() && dual && w.weapon_in_use(o) != Some(item) {
        6
    } else {
        c
    }
}

/// Sender flag argument without the fillers' 0x9D action 0x13 (the
/// cube spill's).
pub const NO_FILLERS: u32 = 0x20;
/// Added to a filler's flag argument.
pub const FILLER_FLAG: u32 = 0x8;

/// Builds a 0x9C / 0x9D for `item` with the sender's flag argument
/// `flags` and, for a socketed item when `flags` lacks 0x20, one 0x9D
/// action 0x13 per filler after it ("Bit stream":
/// owner = the parent item, flag argument `flags` | 0x8).
fn item_message<W: MoveWorld>(
    w: &W,
    message: u8,
    action: u8,
    owner: Owner,
    item: Guid,
    flags: u32,
    page: u8,
) -> Result<Vec<Vec<u8>>, MoveFatal> {
    let bits = w.item_bits(item, flags, page);
    let cat = category(w, item);
    let first = if message == 0x9C {
        layouts::item_world(action, cat, item, &bits)?
    } else {
        layouts::item_owned(action, cat, item, owner.ty, owner.guid, &bits)?
    };
    let mut out = vec![first];
    if flags & NO_FILLERS != 0 || w.item_flags(item) & iflag::SOCKETED == 0 {
        return Ok(out);
    }
    for f in w.fillers(item) {
        let fb = w.item_bits(f, flags | FILLER_FLAG, w.page(f));
        out.push(layouts::item_owned(
            0x13,
            category(w, f),
            f,
            Owner::ITEM,
            item,
            &fb,
        )?);
    }
    Ok(out)
}

/// Unit +0xC8 bit 2: the vendor item flag.
pub const VENDOR_ITEM: u32 = 0x4;
/// 0x9C action of a store item shown to its client (sent with 0x38).
pub const STORE_SHOWN: u8 = 0x0B;

/// The store check with 0x38: S→C 0x9C action 0x0B
/// of `item` with its store bit stream ([`MovePending::store_item_bits`],
/// alt-code for an unidentified quality 4–9 item), then its fillers as
/// [`item_message`] sends them.
///
/// [`MovePending::store_item_bits`]: super::seams::MovePending::store_item_bits
pub fn store_item_message<W: MoveWorld>(w: &W, item: Guid) -> Result<Vec<Vec<u8>>, MoveFatal> {
    let page = w.page(item);
    let bits = w.store_item_bits(item, page);
    let mut out = vec![layouts::item_world(
        STORE_SHOWN,
        category(w, item),
        item,
        &bits,
    )?];
    if w.item_flags(item) & iflag::SOCKETED == 0 {
        return Ok(out);
    }
    for f in w.fillers(item) {
        let fb = w.item_bits(f, FILLER_FLAG, w.page(f));
        out.push(layouts::item_owned(
            0x13,
            category(w, f),
            f,
            Owner::ITEM,
            item,
            &fb,
        )?);
    }
    Ok(out)
}

/// The dispatcher for one item of `inv`'s update list,
/// as seen by the client of `client` (a player). Returns the messages in
/// send order.
pub fn dispatch<W: MoveWorld>(
    w: &mut W,
    client: Guid,
    inv: Owner,
    item: Guid,
) -> Result<Vec<Vec<u8>>, MoveFatal> {
    let client = Owner::player(client);
    let mut out = w.store_messages(client, item);
    // The vendor-item store check ends the walk.
    // TODO(inventory-moves): only the client trading with the owner
    // gets the store message; the seam has no trading-client test.
    if w.update_bits(Owner::item(item)) & VENDOR_ITEM != 0 {
        return Ok(out);
    }
    let is_owner = inv == client;
    let cmdf = w.cmd_flags(item);
    let itf = w.item_flags(item);
    let m = w.mode(item);
    for r in &ITEM_ACTIONS {
        let v = match r.test {
            Test::Cmd => cmdf,
            Test::Item => itf,
        };
        if v & r.flags == 0 {
            continue;
        }
        let cond = match r.cond {
            Cond::None => true,
            Cond::NotBroken => itf & iflag::BROKEN == 0,
            Cond::UpdateStats => {
                m <= mode::BELT
                    && itf & (iflag::BROKEN | iflag::REPAIRED) == 0
                    && !(is_owner && itf & iflag::NO_UPDATE != 0)
            }
        };
        if !cond {
            continue;
        }
        // A row matches only when its `to` test passes too; a row whose
        // `to` excludes this client lets the walk go on, except the
        // item-flag rows 18 and 19, which end it with nothing sent.
        let to = match r.to {
            To::Owner => is_owner,
            To::All => true,
            To::OwnerOrMode1 => is_owner || m == mode::EQUIPPED,
        };
        if !to {
            if r.message == 0x7D {
                break;
            }
            continue;
        }
        if r.message == 0x7D {
            out.push(layouts::item_state(
                inv.ty,
                inv.guid,
                item,
                r.action,
                itf & r.action,
            ));
        } else {
            if r.order == 1 {
                // 0x9C action 1 first sets the item's x, y to 0.
                w.set_pos(Owner::item(item), 0, 0);
            }
            // The dispatcher's sends pass the flag argument 0. Row
            // 4's sender shows the stored page;
            // measured: a 0x19 pick-up's 0x9D action 5 carries
            // page 0 (stream page + 1 = 1), `facts/items/a1-town-potions-low.tsv`
            // n 1, `a1-town-item-moves.tsv` n 32.
            let page = if r.sender == 0x0053D010 {
                w.stored_page(item)
            } else {
                w.page(item)
            };
            out.extend(item_message(
                w,
                r.message,
                r.action as u8,
                inv,
                item,
                0,
                page,
            )?);
        }
        break;
    }
    Ok(out)
}

/// The update-list pass of `player` for the
/// client of `client`: for each node of the player's update list, in
/// order, the item looked up by GUID (missing → skipped) and the
/// dispatcher, then the dispatcher over the item's own update
/// list when it has +0xC8 bit 0; then the hireling owner pass. The join
/// sends it as the player's item messages.
pub fn update_list_pass<W: MoveWorld>(
    w: &mut W,
    client: Guid,
    player: Guid,
) -> Result<Vec<Vec<u8>>, MoveFatal> {
    let p = Owner::player(player);
    let mut out = Vec::new();
    for g in w.update_list(p) {
        if !w.unit_exists(Owner::item(g)) {
            continue;
        }
        out.extend(dispatch(w, client, p, g)?);
        let io = Owner::item(g);
        if w.update_bits(io) & 1 != 0 {
            for h in w.update_list(io) {
                out.extend(dispatch(w, client, io, h)?);
            }
        }
    }
    w.hireling_owner_pass(p);
    Ok(out)
}

/// The update-list pass of a hireling `merc`
/// for the client of `client` (its owner's player): the dispatcher over
/// each node of the merc's update list (and over the update list of an
/// item with +0xC8 bit 0), then, when the list was not empty, the
/// hireling's inventory pass with (…, 1) whose send ends with S→C
/// 0x48 (type 1, the hireling's GUID) and the
/// empty. Recorded: `hire-items-kashya` frames 34 and 50
/// (0x9D action 6 / 8 with owner type 1, then 0x48).
pub fn merc_update_pass<W: MoveWorld>(
    w: &mut W,
    client: Guid,
    merc: Guid,
) -> Result<Vec<Vec<u8>>, MoveFatal> {
    let m = Owner {
        ty: Owner::MONSTER,
        guid: merc,
    };
    let list = w.update_list(m);
    let mut out = Vec::new();
    for &g in &list {
        if !w.unit_exists(Owner::item(g)) {
            continue;
        }
        out.extend(dispatch(w, client, m, g)?);
        let io = Owner::item(g);
        if w.update_bits(io) & 1 != 0 {
            for h in w.update_list(io) {
                out.extend(dispatch(w, client, io, h)?);
            }
        }
    }
    if !list.is_empty() {
        out.push(layouts::relator2(Owner::MONSTER, 0, merc));
    }
    Ok(out)
}

/// Player unit update for the client of
/// `client`: when `player` has +0xC8 bit 0, the update-list pass,
/// then S→C 0x47 and 0x48 for the player.
pub fn player_update<W: MoveWorld>(
    w: &mut W,
    client: Guid,
    player: Guid,
) -> Result<Vec<Vec<u8>>, MoveFatal> {
    let p = Owner::player(player);
    if w.update_bits(p) & 1 == 0 {
        return Ok(Vec::new());
    }
    let mut out = update_list_pass(w, client, player)?;
    out.push(layouts::relator1(Owner::PLAYER, player));
    out.push(layouts::relator2(Owner::PLAYER, 0, player));
    // The per-item reset and the list free are the room clean-up's
    // ([`room_cleanup`]).
    Ok(out)
}

/// Command flags the per-item reset clears (table, 21
/// entries; 0x1 is not among them).
pub const RESET_CMD_FLAGS: [u32; 21] = [
    0x2, 0x4, 0x8, 0x10, 0x20, 0x40, 0x80, 0x100, 0x40000, 0x400, 0x800, 0x1000, 0x2000, 0x200,
    0x4000, 0x8000, 0x10000, 0x20000, 0x80000, 0x100000, 0x200000,
];
/// Item flags the per-item reset clears (table, 8 entries).
pub const RESET_ITEM_FLAGS: [u32; 8] = [0x20, 0x2, 0x8, 0x80, 0x40, 0x1, 0x200, 0x40000];
/// Unit flags (+0xC4) the room clean-up clears.
pub const CLEANUP_UNIT_FLAGS: u32 = 0x1 | 0x10 | 0x400 | 0x8000;
/// Update bits (+0xC8) the room clean-up clears.
pub const CLEANUP_UPDATE_BITS: u32 = 0x800 | 0x1000 | 0x10000 | 0x200000;
/// Update bits (+0xC8) the per-item reset clears.
pub const RESET_ITEM_BITS: u32 = 0x4 | 0x10;
/// Command flag 0x1: the item is removed by the reset.
pub const CMD_REMOVE: u32 = 0x1;

fn mask(flags: &[u32]) -> u32 {
    flags.iter().fold(0, |a, &f| a | f)
}

/// Per-item reset.
pub fn item_reset<W: MoveWorld>(w: &mut W, item: Guid) {
    let io = Owner::item(item);
    let b = w.update_bits(io);
    w.set_update_bits(io, b & !RESET_ITEM_BITS);
    let c = w.cmd_flags(item);
    w.set_cmd_flags(item, c & !mask(&RESET_CMD_FLAGS));
    let f = w.item_flags(item);
    w.set_item_flags(item, f & !mask(&RESET_ITEM_FLAGS));
}

/// Update-list reset with (game, unit) (D2MOO
/// `D2GAME_INVMODE_Last`): a unit without an inventory → nothing; else
/// the owner refresh with 0 (+0xC8 bit 0 cleared; bit 1, "save pending",
/// stays), then for each listed item found by GUID: body location 0 for
/// command flags 0x10 / 0x4000 (or 0x20 with item flag 0x80), the
/// per-item reset, the item's own update list (+0xC8 bit 0) reset and
/// freed, and the removal of an item with command flag 0x1. Last the
/// unit's update list is freed.
pub fn update_list_reset<W: MoveWorld>(w: &mut W, unit: Owner) {
    if !w.has_inventory(unit) {
        return;
    }
    let b = w.update_bits(unit);
    w.set_update_bits(unit, b & !1);
    for g in w.update_list(unit) {
        if !w.unit_exists(Owner::item(g)) {
            continue;
        }
        let c = w.cmd_flags(g);
        if c & (super::cmd::UNEQUIP | super::cmd::AUTO_UNEQUIP) != 0
            || (c & super::cmd::SWAP_BODY != 0 && w.item_flags(g) & iflag::SWAP_OUT != 0)
        {
            w.set_body_loc(g, 0);
        }
        item_reset(w, g);
        let io = Owner::item(g);
        let ib = w.update_bits(io);
        if ib & 1 != 0 {
            w.set_update_bits(io, ib & !1);
            for h in w.update_list(io) {
                if w.unit_exists(Owner::item(h)) {
                    item_reset(w, h);
                }
            }
            w.update_list_free(io);
        }
        if w.cmd_flags(g) & CMD_REMOVE != 0 {
            w.free_item(g);
        }
    }
    w.update_list_free(unit);
}

/// Room update clean-up with (game, unit): unit flags 0x1, 0x10, 0x400, 0x8000 and update bits
/// 0x800, 0x1000, 0x10000, 0x200000 cleared, then [`update_list_reset`].
pub fn room_cleanup<W: MoveWorld>(w: &mut W, unit: Owner) {
    let f = w.unit_flags(unit);
    w.set_unit_flags(unit, f & !CLEANUP_UNIT_FLAGS);
    let b = w.update_bits(unit);
    w.set_update_bits(unit, b & !CLEANUP_UPDATE_BITS);
    update_list_reset(w, unit);
}

/// Item part of the per-unit update (the tick's client pass). Unit flag
/// 0x10 set (new, not yet announced): part 1, the
/// unit-add message ([`announce_item`]). Otherwise part 2: the
/// item unit update, which runs [`ground_update`] only when
/// unit flag 0x1 (changed) is set. At most one message per call.
pub fn item_unit_update<W: MoveWorld>(w: &W, item: Guid) -> Result<Option<Vec<u8>>, MoveFatal> {
    let flags = w.unit_flags(Owner::item(item));
    if flags & uflag::NOT_ANNOUNCED != 0 {
        return announce_item(w, item).map(Some);
    }
    if flags & uflag::CHANGED == 0 {
        return Ok(None);
    }
    ground_update(w, item)
}

/// Part 1, the item case of the unit-add messages:
/// mode 3 with unit flag 0x1000 → 0x9C action 2 (dropped);
/// otherwise → 0x9C action 0 (new).
pub fn announce_item<W: MoveWorld>(w: &W, item: Guid) -> Result<Vec<u8>, MoveFatal> {
    let dropped = w.unit_flags(Owner::item(item)) & uflag::DROPPED != 0;
    announce_item_as(w, item, dropped)
}

/// [`announce_item`] with the unit flag 0x1000 given by the caller
/// (`dropped`): a host whose pass runs after the room clean-up has
/// cleared the flag keeps it itself.
pub fn announce_item_as<W: MoveWorld>(
    w: &W,
    item: Guid,
    dropped: bool,
) -> Result<Vec<u8>, MoveFatal> {
    let action = if dropped && w.mode(item) == mode::GROUND {
        2
    } else {
        0
    };
    let bits = w.item_bits(item, 0, w.page(item));
    layouts::item_world(action, category(w, item), item, &bits)
}

/// Ground item update (part 2): a mode-3 item without
/// unit flag 0x10 (already announced) sends 0x9C action 2 (unit flag
/// 0x1000) or 3. The caller is [`item_unit_update`].
pub fn ground_update<W: MoveWorld>(w: &W, item: Guid) -> Result<Option<Vec<u8>>, MoveFatal> {
    let flags = w.unit_flags(Owner::item(item));
    if flags & uflag::NOT_ANNOUNCED != 0 {
        return Ok(None);
    }
    ground_update_as(w, item, flags & uflag::DROPPED != 0)
}

/// [`ground_update`] of an item known to be announced (unit flag 0x10
/// clear), with the unit flag 0x1000 given by the caller (`dropped`): a
/// host whose pass runs after the room clean-up has cleared both flags
/// reads them before it.
pub fn ground_update_as<W: MoveWorld>(
    w: &W,
    item: Guid,
    dropped: bool,
) -> Result<Option<Vec<u8>>, MoveFatal> {
    if w.mode(item) != mode::GROUND {
        return Ok(None);
    }
    let action = if dropped { 2 } else { 3 };
    let bits = w.item_bits(item, 0, w.page(item));
    Ok(Some(layouts::item_world(
        action,
        category(w, item),
        item,
        &bits,
    )?))
}

/// Direct 0x9D action 5: the page shown as `shown`
/// (the stored page; 3 for the cube spill), item flags OR-ed with
/// `flags`; queued to the owner's client now.
pub fn send_item_page<W: MoveWorld>(
    w: &mut W,
    player: Owner,
    item: Guid,
    flags: u32,
    shown: u8,
) -> Result<(), MoveFatal> {
    for m in item_message(w, 0x9D, 0x05, player, item, flags, shown)? {
        w.send(player, m);
    }
    Ok(())
}

/// Direct 0x9C action 0xE, queued now.
pub fn send_to_belt<W: MoveWorld>(w: &mut W, player: Owner, item: Guid) -> Result<(), MoveFatal> {
    let page = w.page(item);
    for m in item_message(w, 0x9C, 0x0E, player, item, 0, page)? {
        w.send(player, m);
    }
    Ok(())
}

/// A direct 0x9C with action `action` and the sender flag argument
/// `flags`, queued to the player's client now (e.g. 0x9C action 0xF of the
/// belt change, with flag 0x20).
pub fn send_item_world<W: MoveWorld>(
    w: &mut W,
    player: Owner,
    item: Guid,
    action: u8,
    flags: u32,
) -> Result<(), MoveFatal> {
    let page = w.page(item);
    for m in item_message(w, 0x9C, action, player, item, flags, page)? {
        w.send(player, m);
    }
    Ok(())
}
