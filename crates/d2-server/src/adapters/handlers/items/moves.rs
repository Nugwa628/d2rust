//! The item-move intents (C→S 0x16–0x29, 0x50, 0x61, 0x63) and the deferred item
//! messages on the wired host.
//!
//! A handler finds the client's player unit and hands the message to
//! `d2_sim::items::moves::handle` on the inventory wiring
//! (`d2_sim::wiring::inventory::InvDesk`: the inventory model, the unit
//! records and lists, the stat lists and the item store of the host's
//! one economy) through [`WorldHost::moves`]. Every check, result code
//! and state change is the module's; the handler maps the result to a
//! [`ResultCode`] and queues what the module sent "now" (0x63's
//! 0x9D / 0x9C, the 0x42 of a stack merge, the 0x3F of a targeting
//! reset) to the receivers' clients, in send order.
//!
//! The 0x9C / 0x9D / 0x7D messages are not the handler's: they leave in
//! the player unit update of the next client pass.
//! [`update_pass`] runs it after `d2_sim::tick::tick` (see its reading).
//!
//! What no d2-sim module provides (positions and the free-spot search,
//! player data, item use, sockets, hirelings, sounds, the item bit
//! stream) is the host's [`MoveRest`] (`d2_sim::wiring::inventory::InvRest`
//! with its `MovePending` part).

use d2_sim::game::Game;
use d2_sim::items::inventory::InvTables;
use d2_sim::items::moves::{self as sim_moves, mode::GROUND, MoveFatal, MoveUnits, Owner, HANDLED};
use d2_sim::tick::EventDispatch;
use d2_sim::units::lifecycle::LifecycleHooks;
use d2_sim::units::sound::sound_message;
use d2_sim::units::UnitId;
use d2_sim::wiring::economy::Economy;
use d2_sim::wiring::inventory::swap::{SwitchSide, SwitchedSets};
use d2_sim::wiring::inventory::{InvDesk, InvRest, InvState};

pub mod preview_skills;

use super::super::super::SimGame;
use super::super::world::{WorldError, WorldFault, WorldHost};
use super::result_code;
use crate::seams::{ClientId, Intents, MessageSink, ResultCode};

/// Every item-move C→S id: (id, message name, design note). Exactly
/// `d2_sim::items::moves::HANDLED`, in id order.
pub const MOVE_IDS: &[(u8, &str, &str)] = &[
    (0x16, "PickItem", "specs/items/inventory-moves.md §7.1"),
    (0x17, "DropItem", "specs/items/inventory-moves.md §7.2"),
    (
        0x18,
        "InsertItemInBuffer",
        "specs/items/inventory-moves.md §7.3",
    ),
    (
        0x19,
        "RemoveItemFromBuffer",
        "specs/items/inventory-moves.md §7.4",
    ),
    (0x1A, "EquipItem", "specs/items/inventory-moves.md §7.5"),
    (
        0x1B,
        "Swap2HandedItem",
        "specs/items/inventory-moves.md §7.6",
    ),
    (
        0x1C,
        "RemoveBodyItem",
        "specs/items/inventory-moves.md §7.7",
    ),
    (
        0x1D,
        "SwapCursorWithBody",
        "specs/items/inventory-moves.md §7.8",
    ),
    (0x1E, "Swap1HWith2H", "specs/items/inventory-moves.md §7.9"),
    (
        0x1F,
        "SwapCursorBufferItem",
        "specs/items/inventory-moves.md §7.10",
    ),
    (0x20, "UseGridItem", "specs/items/inventory-moves.md §7.11"),
    (0x21, "StackItems", "specs/items/inventory-moves.md §7.12"),
    (0x22, "UnstackItems", "specs/items/inventory-moves.md §7.13"),
    (0x23, "ItemToBelt", "specs/items/inventory-moves.md §7.14"),
    (0x24, "ItemFromBelt", "specs/items/inventory-moves.md §7.15"),
    (
        0x25,
        "SwitchBeltItem",
        "specs/items/inventory-moves.md §7.16",
    ),
    (0x26, "UseBeltItem", "specs/items/inventory-moves.md §7.17"),
    (
        0x27,
        "UseItemAction",
        "specs/items/inventory-moves.md §7.18",
    ),
    (0x28, "SocketItem", "specs/items/inventory-moves.md §7.19"),
    (0x29, "ScrollToBook", "specs/items/inventory-moves.md §7.20"),
    (0x50, "DropGold", "specs/items/inventory-moves.md §7.22"),
    (0x61, "MercItem", "specs/items/inventory-moves.md §7.23"),
    (
        0x63,
        "ItemToBeltShift",
        "specs/items/inventory-moves.md §7.24",
    ),
];

/// An item-move id (one [`MOVE_IDS`] row).
pub fn is_move_id(id: u8) -> bool {
    HANDLED.iter().any(|&(i, _)| i == id)
}

/// The inventory seams without a d2-sim provider, as the host holds
/// them: [`InvRest`] (with `MovePending`), plus the take of what
/// `MovePending::send` queued.
pub trait MoveRest: InvRest {
    /// The messages `MovePending::send` queued since the last take, in
    /// send order: (receiving unit, bytes).
    fn take_sent(&mut self) -> Vec<(Owner, Vec<u8>)>;

    /// The places of the game's players and items at the start of a
    /// move call, and the game's item format, for a rest that holds no
    /// positions of its own (the play preview's, [`StagedPlace`]).
    /// Default: ignored.
    fn stage(&mut self, _places: &[StagedPlace], _item_format: u16) {}

    /// The sub-tile boxes of the rooms a drop can land in (the players'
    /// rooms and the rooms next to them), for a rest that holds no room
    /// geometry of its own: a ground spot belongs to the room whose box
    /// contains it (half-open, like `DrlgWorld::find_room`).
    /// Default: ignored.
    fn stage_rooms(&mut self, _rooms: &[(d2_sim::units::RoomId, d2_sim::drlg::TileRect)]) {}

    /// The players' quest flag records (current difficulty) at the start
    /// of a move call, for a rest that holds no quest record of its own
    /// (the grid-item use). Default: ignored.
    fn stage_quest_flags(&mut self, _flags: &[(Owner, d2_sim::world::quests::QuestFlags)]) {}

    /// The quest flag writes the call made (`MovePending::set_quest_flag`)
    /// since the last take: (player, quest, flag, on). Default: none.
    fn take_quest_flag_writes(&mut self) -> Vec<(Owner, u8, u8, bool)> {
        Vec::new()
    }

    /// The ground items the call picked up, (player, item) in order
    /// (`MovePending::quest_item_picked`, hook ITEMPICKEDUP). Default: none.
    fn take_picked_items(&mut self) -> Vec<(Owner, d2_sim::items::moves::Guid)> {
        Vec::new()
    }

    /// Each player's first golem pet (pet type 3) at the start of a
    /// move call, and the Blood Golem's `Param6` (monstats row 290
    /// `Skill1`), for a rest that holds no pet lists of its own: the
    /// potion share `d2_sim::wiring::inventory::potion::golem_share`.
    /// Default: ignored.
    fn stage_pets(&mut self, _golems: &[(Owner, Owner)], _blood_golem_param6: Option<i32>) {}

    /// The players' skill lists, lent for one move call (the ranged-throw
    /// test reads `tables`' item type equivalence).
    fn stage_skills(&mut self, _stage: preview_skills::SkillStage, _tables: &InvTables) {}

    /// The lent lists back, after the call.
    fn take_skills(&mut self) -> Option<preview_skills::SkillStage> {
        None
    }

    /// Messages made after the call, sent with the call's own.
    fn queue_sent(&mut self, _sent: Vec<(Owner, Vec<u8>)>) {}
}

/// Where a player or an item is when an item-move call starts: its
/// owner key, position in sub-tiles and room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StagedPlace {
    /// The unit's owner key.
    pub owner: Owner,
    /// Position in sub-tiles.
    pub pos: (i32, i32),
    /// The room the unit is in, if any.
    pub room: Option<d2_sim::units::RoomId>,
    /// The unit has a used skill.
    pub used_skill: bool,
}

/// The item-move part of a game's world host: the inventory tables
/// (`InvTables::from_fixed`), the inventory state (one inventory per
/// owner unit, the item data fields this spec owns, the wiring's
/// errors) and the seams without a provider. The units, stats and items
/// are the host's economy.
pub struct InvParts {
    /// The inventory tables.
    pub tables: InvTables,
    /// `InvState::errors` collects the wiring's provider errors; the
    /// host reads them (they are not handler results).
    pub state: InvState,
    /// The seams without a d2-sim provider.
    pub rest: Box<dyn MoveRest + Send + Sync>,
}

impl InvParts {
    /// Item-move parts over `tables` and the host's `rest`, with equip
    /// rules on and worn items linking their stats to the wearer.
    pub fn new(tables: InvTables, rest: Box<dyn MoveRest + Send + Sync>) -> Self {
        let mut state = InvState::new();
        state.equip_rules = true;
        // Worn items attach their stats to the wearer (`item_link`).
        state.link_item_stats = true;
        Self {
            tables,
            state,
            rest,
        }
    }

    /// The inventory desk of one call over the host's economy
    /// (`InvDesk::new` fills the item data copies): the item moves, the
    /// update pass, and the vendor and cube adapters all run on it.
    pub fn desk<'d, 'a, H: LifecycleHooks>(
        &'d mut self,
        econ: &'d mut Economy<'a, H>,
    ) -> InvDesk<'d, 'a, H, dyn MoveRest + Send + Sync> {
        let InvParts {
            tables,
            state,
            rest,
        } = self;
        InvDesk::new(econ, tables, state, rest.as_mut())
    }
}

/// One call on the inventory wiring over the host's economy.
pub trait MoveCall {
    /// What the call returns.
    type Out;
    /// Run the call on the economy and the inventory parts.
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out;
    /// Messages made inside the call by a quest hook that runs after it
    /// (ITEMPICKEDUP): put ahead of the call's own messages when the output
    /// carries them; the rest is handed back for the host's outbox.
    fn front_sent(_out: &mut Self::Out, msgs: Vec<(UnitId, Vec<u8>)>) -> Vec<(UnitId, Vec<u8>)> {
        msgs
    }
}

/// What the rest sent during a call, with the receiving units looked up
/// (`None`: a GUID without a unit; nothing is sent to it).
pub(crate) fn take_sent<H: LifecycleHooks>(
    d: &mut InvDesk<'_, '_, H, dyn MoveRest + Send + Sync>,
) -> Vec<(Option<UnitId>, Vec<u8>)> {
    let sent = d.rest.take_sent();
    sent.into_iter().map(|(o, b)| (d.unit_of(o), b)).collect()
}

/// One item-move message.
pub(crate) struct MoveRun<'m> {
    pub(crate) player: UnitId,
    pub(crate) msg: &'m [u8],
}

pub(crate) type MoveOut = (
    Option<Result<u32, MoveFatal>>,
    Vec<(Option<UnitId>, Vec<u8>)>,
    Vec<(UnitId, UnitId, bool)>,
);

/// The item use inside the handler runs the Town Portal cast on the
/// economy's hooks (`LifecycleHooks::town_portal_cast`), so a 0x20 /
/// 0x26 / 0x27 charges the scroll or tome only when the cast made the
/// portal pair.
impl MoveCall for MoveRun<'_> {
    type Out = MoveOut;
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> MoveOut {
        let mut d = parts.desk(econ);
        let guid = d.guid_of(self.player);
        let r = sim_moves::handle(&mut d, guid, self.msg);
        d.flush_equip();
        let walks = d.take_item_walks();
        (r, take_sent(&mut d), walks)
    }
    fn front_sent(out: &mut MoveOut, msgs: Vec<(UnitId, Vec<u8>)>) -> Vec<(UnitId, Vec<u8>)> {
        out.1
            .splice(0..0, msgs.into_iter().map(|(u, b)| (Some(u), b)));
        Vec::new()
    }
}

/// The item part of the corpse creations the death queued
/// (`ActionHooks::death.loot`): each corpse
/// gets an inventory, then copies of the player's cursor and body items
/// go onto it ([`sim_moves::ground::corpse_fill`]). The result carries what the
/// rest sent.
pub struct CorpseFillRun {
    /// (player, corpse) with their GUIDs and the corpse's class.
    pub pairs: Vec<(Owner, Owner, u32)>,
    /// (player, amount) gold drops of the death penalty.
    pub gold: Vec<(Owner, i32)>,
}

impl MoveCall for CorpseFillRun {
    type Out = (Vec<MoveFatal>, Vec<(Option<UnitId>, Vec<u8>)>);
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        let mut faults = Vec::new();
        let mut d = parts.desk(econ);
        // Gold piles near the player's death spot, the amount
        for (p, amount) in self.gold {
            sim_moves::ground::gold_piles(&mut d, p, amount, sim_moves::MAX_PILES);
        }
        for (p, c, class) in self.pairs {
            if let Some(cu) = d.unit_of(c) {
                d.state.add_inventory(
                    cu,
                    d2_sim::items::inventory::UnitKind::Player { class: class as u8 },
                    c.guid,
                );
            }
            if let Err(e) = sim_moves::ground::corpse_fill(&mut d, p, c) {
                faults.push(e);
            }
        }
        (faults, take_sent(&mut d))
    }
}

/// C→S 0x60 SwapWeapons: the handler gates (the player handler's) and
/// the body ([`InvDesk::swap_weapon_sets`],
/// [`InvDesk::weapon_switch_sides`]). A host without the inventory parts
/// keeps the player handler's stub.
pub const SWAP_WEAPONS: u8 = 0x60;

/// S→C 0x97 WeaponSwitch (one byte).
const WEAPON_SWITCH: u8 = 0x97;

struct SwapRun {
    player: UnitId,
}

impl MoveCall for SwapRun {
    type Out = (Option<SwitchedSets>, u32, Vec<(Option<UnitId>, Vec<u8>)>);
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        let mut d = parts.desk(econ);
        let owner = d.owner_of(self.player);
        let sets = owner.and_then(|o| d.swap_weapon_sets(o));
        (sets, owner.map_or(0, |o| o.guid), take_sent(&mut d))
    }
}

/// Steps 8, 10 and 11 of the switch on the lists the previous call
/// returned (its stat 97 / 107 callback has run, so the new hands'
/// skills exist), with the messages the rest queued after that call: the
/// callback's S→C 0x21.
struct SideRun {
    player: UnitId,
    saved: [(i32, i32); 2],
}

impl MoveCall for SideRun {
    type Out = (Vec<(Option<UnitId>, Vec<u8>)>, Option<[SwitchSide; 2]>);
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        let mut d = parts.desk(econ);
        let sent = take_sent(&mut d);
        let sides = d
            .owner_of(self.player)
            .map(|o| d.weapon_switch_sides(o, self.saved));
        (sent, sides)
    }
}

/// A move call that only takes the messages the rest queued after the
/// previous call: the stat 97 / 107 callback's S→C 0x21, which the
/// preview runs once the call returns.
struct DrainRun;

impl MoveCall for DrainRun {
    type Out = Vec<(Option<UnitId>, Vec<u8>)>;
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        take_sent(&mut parts.desk(econ))
    }
}

fn swap_weapons<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    client: ClientId,
    msg: &[u8],
    size: usize,
    out: &mut dyn MessageSink,
) -> Option<ResultCode> {
    // The gates (classic → 3; used skill or dead → 0) are the player
    // handler's, which answers `None` once they pass (it has no
    // provider for the switch itself).
    if let Some(code) = super::super::player::handle(sim, client, msg, size, out) {
        return Some(code);
    }
    let player = sim.player_of(client)?;
    let (game, events) = (&mut sim.game, &mut sim.events);
    let (sets, guid, mut sent) = sim.world.moves(game, events, SwapRun { player })?;
    let ok = sets.is_some_and(|s| s.ok);
    // The new hands' stat links run the stat 97 / 107 callback inside the
    // switch's inventory pass, so its 0x21 go before S→C 0x97.
    let (game, events) = (&mut sim.game, &mut sim.events);
    let mut sides = None;
    let early = match sets {
        Some(s) if s.ok => sim
            .world
            .moves(
                game,
                events,
                SideRun {
                    player,
                    saved: s.saved,
                },
            )
            .map(|(early, s)| {
                sides = s;
                early
            }),
        _ => sim.world.moves(game, events, DrainRun),
    };
    if let Some(mut early) = early {
        early.append(&mut sent);
        sent = early;
    }
    for (unit, bytes) in sent {
        if let Some(c) = unit.and_then(|u| sim.client_of(u)) {
            // A full queue is the host's fault elsewhere; the swap stands.
            let _ = out.queue(c, &bytes);
        }
    }
    if !ok {
        // A failed switch answers 3 (Malformed) and sends no 0x97.
        return Some(ResultCode::Malformed);
    }
    // Step 9: S→C 0x97 to the player's client; then steps 10–11: the
    // direct 0x23 of each side that selects, and the two 0x23 records
    // the next update sends after 0x47 / 0x48.
    let _ = out.queue(client, &[WEAPON_SWITCH]);
    let (direct, records) = match sides {
        Some(sides) => sim
            .world
            .weapon_switched(&mut sim.events, player, guid, sides),
        None => (Vec::new(), Vec::new()),
    };
    for bytes in direct {
        let _ = out.queue(client, &bytes);
    }
    for bytes in records {
        sim.switch_events.push((client, bytes));
    }
    Some(ResultCode::Done)
}

/// The handler of an item-move id after the dispatcher's gate and size
/// check. `None`: not an item-move id, no player, or a host without the
/// inventory parts, so the caller keeps its stub. A fatal inconsistency
/// (`MoveFatal`) or a queueing failure is recorded with the host
/// ([`WorldHost::fault`]) and the handler returns
/// [`ResultCode::Malformed`], as the world handlers do.
pub fn handle<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    client: ClientId,
    msg: &[u8],
    size: usize,
    out: &mut dyn MessageSink,
) -> Option<ResultCode> {
    let id = *msg.first()?;
    if id == SWAP_WEAPONS && size == 1 {
        return swap_weapons(sim, client, msg, size, out);
    }
    // C→S 0x13 with unit type 4 (a ground item) is the type-4 case of
    // 0x16 with cursor flag 0, so it runs as that message.
    let as_pick;
    let msg = if id == 0x13 && size == 9 && msg.len() >= 9 && msg[1..5] == 4u32.to_le_bytes() {
        let mut m = vec![0x16];
        m.extend_from_slice(&msg[1..9]);
        m.extend_from_slice(&[0; 4]);
        as_pick = m;
        &as_pick[..]
    } else {
        msg
    };
    let (id, size) = if msg.len() == 13 && id == 0x13 {
        (0x16, 13)
    } else {
        (id, size)
    };
    if !is_move_id(id) {
        return None;
    }
    let player = sim.player_of(client)?;
    // Every item-move id has a fixed size ≤ 17 (`client-messages.tsv`).
    let msg = &msg[..size.min(msg.len())];
    let (game, events) = (&mut sim.game, &mut sim.events);
    let (run, sent, walks) = sim.world.moves(game, events, MoveRun { player, msg })?;
    // A pick-up out of reach runs the player to the item.
    for w in walks {
        sim.world.item_walk(game, events, w);
    }
    let mut faults = Vec::new();
    for (unit, bytes) in sent {
        // A unit without a client receives nothing.
        if let Some(c) = unit.and_then(|u| sim.client_of(u)) {
            // A room delete notice only prepends a record to the room's
            // delete list; the tick sends it. Type 4: an item; type 0: a
            // corpse taken back.
            if bytes.len() == 6 && bytes[0] == 0x0A && matches!(bytes[1], 0 | 4) {
                sim.deferred_room_deletes.push((c, bytes));
            } else if let Err(e) = out.queue(c, &bytes) {
                faults.push(WorldError::from(e));
            }
        }
    }
    let result = match run? {
        Ok(r) => result_code(r),
        Err(e) => {
            faults.push(WorldError::Move(e));
            ResultCode::Malformed
        }
    };
    let failed = !faults.is_empty();
    for error in faults {
        sim.world.fault(WorldFault { client, id, error });
    }
    Some(if failed {
        ResultCode::Malformed
    } else {
        result
    })
}

/// One client's part of the update pass: the client's own player (the
/// owner test of the item update) and the players whose unit update it processes.
#[derive(Clone)]
struct Receiver {
    client: ClientId,
    own: UnitId,
    players: Vec<UnitId>,
    /// Item units of the client's rooms not yet announced to it (the
    /// ground items' part of the unit update).
    ground: Vec<UnitId>,
    /// Players whose item messages / sound already went out in the tick's
    /// queue walk ([`player_marked`]).
    items_done: Vec<UnitId>,
    sound_done: Vec<UnitId>,
}

/// The update pass of one tick.
#[derive(Clone)]
struct UpdateRun {
    receivers: Vec<Receiver>,
    /// Every player of the pass, for the clean-up.
    players: Vec<UnitId>,
    /// Ground items announced inside the tick's walk ([`GroundRun`]):
    /// their flags clear with the pass's.
    walked: Vec<UnitId>,
}

impl UpdateRun {
    /// The S→C 0x2C of each (receiver, player) of the pass, in pass order
    /// (the sound's target is none or the receiver's player).
    fn sounds(&self, game: &Game) -> Vec<Option<[u8; 8]>> {
        self.receivers
            .iter()
            .flat_map(|r| {
                r.players.iter().map(move |&p| {
                    if r.sound_done.contains(&p) {
                        None
                    } else {
                        sound_message(game, p, r.own)
                    }
                })
            })
            .collect()
    }

    /// Unit flag 0x400 of every player of the pass := 0 (the room
    /// clean-up's sound step, for the players; the tick wiring's
    /// clean-up leaves players' slots to this pass).
    fn clear_sounds(&self, game: &mut Game) {
        for &p in &self.players {
            game.sounds.clear(p);
        }
    }
}

type UpdateOut = (
    Vec<(ClientId, Vec<u8>)>,
    Vec<(ClientId, MoveFatal)>,
    Vec<(ClientId, UnitId)>,
);

impl MoveCall for UpdateRun {
    type Out = UpdateOut;
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> UpdateOut {
        // The sound of each (receiver, player), read before the pass: the
        // item messages do not touch the sound slots.
        let sounds = self.sounds(econ.game);
        let mut d = parts.desk(econ);
        // The items placed on the ground since the last pass: unit flag
        // 0x1000, set by the drop and cleared by the room clean-up after
        // the tick's client pass, so one pass per tick sees each drop
        // once.
        let dropped = d.take_dropped();
        let (mut sent, mut fatal) = (Vec::new(), Vec::new());
        let mut announced = Vec::new();
        let mut sounds = sounds.into_iter();
        for r in &self.receivers {
            let own = d.guid_of(r.own);
            for &p in &r.players {
                let guid = d.guid_of(p);
                if !r.items_done.contains(&p) {
                    match sim_moves::player_update(&mut d, own, guid) {
                        Ok(msgs) => sent.extend(msgs.into_iter().map(|m| (r.client, m))),
                        Err(e) => fatal.push((r.client, e)),
                    }
                }
                // After the item messages and 0x47 / 0x48 comes the
                // player's sound when unit flag 0x400 is set (S→C 0x2C).
                if let Some(m) = sounds.next().flatten() {
                    sent.push((r.client, m.to_vec()));
                }
            }
            // The ground items of the client's rooms, each announced with its
            // unit-add message (S→C 0x9C); D2RUST's own, unverified.
            for &u in &r.ground {
                let guid = d.guid_of(u);
                if d.unit_exists(Owner::item(guid)) && d.mode(guid) == GROUND {
                    // An item dropped in this tick is announced with
                    // action 2 (unit flag 0x1000; a 0x17 drop from the
                    // cursor).
                    let was_dropped = dropped.contains(&u);
                    match sim_moves::announce_item_as(&d, guid, was_dropped) {
                        Ok(m) => sent.push((r.client, m)),
                        Err(e) => fatal.push((r.client, e)),
                    }
                    announced.push((r.client, u));
                }
            }
        }
        // The item case of the room clean-up: item flags 0x20 and 0x2000
        // clear once the item sat in a client's room queue, after the
        // announcing 0x9C carried them.
        for r in &self.receivers {
            for &u in &r.ground {
                if let Some(it) = d.econ.items.get_mut(u) {
                    it.flags &= !0x2020;
                }
            }
        }
        for &u in &self.walked {
            if let Some(it) = d.econ.items.get_mut(u) {
                it.flags &= !0x2020;
            }
        }
        // The clean-up walks the update queues of every room of every
        // act, not only the clients' rooms: an item lies in a room only
        // after a placement that queued it, so the items of every room
        // with an update queue are cleared too (the drops of minions far
        // from the player lose 0x2000 as well).
        let lists = &d.econ.game.lists;
        let in_rooms: Vec<UnitId> = (0..d2_sim::units::lists::ACTS as u8)
            .flat_map(|act| std::iter::successors(lists.room_first(act), |&r| lists.room_next(r)))
            .filter(|&r| lists.room(r).is_some_and(|e| !e.no_update))
            .flat_map(|r| lists.room_units(r))
            .collect();
        for u in in_rooms {
            if let Some(it) = d.econ.items.get_mut(u) {
                it.flags &= !0x2020;
            }
            // The inventory's copy of the item flags (`InvState::items`).
            if let Some(it) = d.state.items.get_mut(&u) {
                it.flags &= !0x2020;
            }
        }
        // The update-list reset of the room clean-up
        // (`InvDesk::update_done`): flags-ex bit 0 cleared (bit 1, "save
        // pending", stays), the per-item resets, the update lists freed.
        // The unit-flag part of the clean-up runs in the tick wiring
        // (`d2_sim::wiring::action::View::room_cleanup`), before this
        // pass.
        for &p in &self.players {
            let Some(o) = d.owner_of(p) else {
                continue;
            };
            if d.update_bits(o) & 1 == 0 {
                continue;
            }
            d.update_done(o);
        }
        // The desk's borrow of the economy ends here.
        self.clear_sounds(econ.game);
        (sent, fatal, announced)
    }
}

/// The deferred item messages of one tick: for each client in
/// client-list order, for each player unit in the client room's adjacent
/// rooms (the rooms in array order, the players of a room in client-list
/// order), the player unit update (`items::moves::player_update`: with
/// flags-ex bit 0, the update-list pass, then 0x47 and 0x48); the messages
/// go to that client. Then the clean-up of every player of the pass.
///
/// Reading: the pass belongs inside the tick's client pass and the
/// clean-up after it; here it runs after `d2_sim::tick::tick`. The tick
/// wiring's per-client unit update sends no item message and its
/// clean-up (flags only) clears no bit this pass reads (flags-ex bits 0 and
/// 1, the item and command flags), so no step of the tick changes what
/// this pass does. Every player is queued for update by its own
/// per-client update (last), so the queue membership test is the room
/// test above. The client's room is read after the tick's room switch:
/// in the tick of a switch the old room's adjacent rooms are walked. The
/// ground items' unit update (`d2_sim::items::moves::item_unit_update`)
/// is not run: it belongs to the per-unit update over the client's
/// rooms, which the tick wiring does not run for items; run here, after
/// the tick, it would find unit flags 0x1 and 0x10 already cleared by
/// the room clean-up.
pub fn update_pass<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    out: &mut dyn MessageSink,
) {
    // An announced item that left the ground (picked up, freed: no room)
    // is forgotten, so its next landing is announced again (a drop from
    // the cursor). The freed pile's removal is the tick's S→C 0x0A.
    let lists = &sim.game.lists;
    sim.announced_ground
        .retain(|&(_, u)| lists.unit(u).is_some_and(|e| e.room().is_some()));
    let clients = sim.clients();
    let players: Vec<(UnitId, Option<d2_sim::units::RoomId>)> = clients
        .iter()
        .filter_map(|&c| sim.player_of(c))
        .map(|p| (p, sim.game.lists.unit(p).and_then(|u| u.room())))
        .collect();
    let mut receivers = Vec::new();
    for &c in &clients {
        let Some(own) = sim.player_of(c) else {
            continue;
        };
        let adjacent = sim
            .sim_client(c)
            .and_then(|id| sim.game.lists.client(id))
            .and_then(|e| e.room)
            .and_then(|r| sim.game.lists.room(r))
            .map(|r| r.adjacent.clone())
            .unwrap_or_default();
        let seen: Vec<UnitId> = adjacent
            .iter()
            .flat_map(|&room| {
                players
                    .iter()
                    .filter(move |&&(_, r)| r == Some(room))
                    .map(|&(p, _)| p)
            })
            .collect();
        let ground: Vec<UnitId> = adjacent
            .iter()
            .filter(|_| sim.announce_ground)
            .flat_map(|&room| sim.game.lists.room_units(room))
            .filter(|&u| {
                sim.game
                    .lists
                    .unit(u)
                    .is_some_and(|e| e.ty == d2_sim::units::UnitType::Item)
                    && !sim.announced_ground.contains(&(c, u))
            })
            .collect();
        receivers.push(Receiver {
            client: c,
            own,
            players: seen,
            ground,
            items_done: sim
                .walk_player_items
                .iter()
                .filter(|&&(cl, _)| cl == c)
                .map(|&(_, p)| p)
                .collect(),
            sound_done: sim
                .walk_player_sound
                .iter()
                .filter(|&&(cl, _)| cl == c)
                .map(|&(_, p)| p)
                .collect(),
        });
    }
    sim.walk_player_items.clear();
    sim.walk_player_sound.clear();
    let walked: Vec<UnitId> = std::mem::take(&mut sim.walk_announced)
        .into_iter()
        .map(|(_, u)| u)
        .collect();
    let run = UpdateRun {
        receivers,
        players: players.iter().map(|&(p, _)| p).collect(),
        walked,
    };
    let (game, events) = (&mut sim.game, &mut sim.events);
    let sound_only = run.clone();
    let Some((sent, fatal, announced)) = sim.world.moves(game, events, run) else {
        // A host without inventory parts: no item messages; the players'
        // sounds still leave and are cleared.
        let mut sounds = sound_only.sounds(&sim.game).into_iter();
        for r in &sound_only.receivers {
            for _ in &r.players {
                let Some(m) = sounds.next().flatten() else {
                    continue;
                };
                if let Err(e) = out.queue(r.client, &m) {
                    sim.tick_faults.push((r.client, WorldError::from(e)));
                }
            }
        }
        sound_only.clear_sounds(&mut sim.game);
        return;
    };
    for (c, bytes) in sent {
        if let Err(e) = out.queue(c, &bytes) {
            sim.tick_faults.push((c, WorldError::from(e)));
        }
    }
    for (c, e) in fatal {
        sim.tick_faults.push((c, WorldError::Move(e)));
    }
    // The queued 0x23 event records of a weapon switch follow the
    // player's 0x47 / 0x48.
    for (c, bytes) in std::mem::take(&mut sim.switch_events) {
        if let Err(e) = out.queue(c, &bytes) {
            sim.tick_faults.push((c, WorldError::from(e)));
        }
    }
    sim.announced_ground.extend(announced);
}

#[cfg(test)]
pub(crate) mod tests;

/// The announcement of one ground item at its place in the client pass's
/// queue walk (the unit-add 0x9C of the per-unit update).
struct GroundRun {
    item: UnitId,
}

impl MoveCall for GroundRun {
    type Out = Option<Result<Vec<u8>, MoveFatal>>;
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        let d = parts.desk(econ);
        let guid = d.guid_of(self.item);
        if !(d.unit_exists(Owner::item(guid)) && d.mode(guid) == GROUND) {
            return None;
        }
        let was_dropped = d.was_dropped(self.item);
        Some(sim_moves::announce_item_as(&d, guid, was_dropped))
    }
}

/// The host's answer to a ground-item mark of the tick's client pass
/// ([`d2_sim::wiring::action::GROUND_ITEM_MARK`]): the item's 0x9C to the
/// marked client now, so it stands where the queue walk put it. Only the
/// play host's pass announces ground items ([`SimGame::announce_ground`]).
pub fn announce_marked<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    out: &mut dyn MessageSink,
    receiver: UnitId,
    guid: u32,
) {
    use d2_sim::units::UnitType;
    if !sim.announce_ground {
        return;
    }
    let Some(c) = sim.client_of(receiver) else {
        return;
    };
    let Some(item) = sim.game.lists.find_unit(UnitType::Item, guid) else {
        return;
    };
    if sim.announced_ground.contains(&(c, item))
        || sim.game.lists.unit(item).and_then(|e| e.room()).is_none()
    {
        return;
    }
    let (game, events) = (&mut sim.game, &mut sim.events);
    let Some(Some(r)) = sim.world.moves(game, events, GroundRun { item }) else {
        return;
    };
    match r {
        Ok(m) => {
            if let Err(e) = out.queue(c, &m) {
                sim.tick_faults.push((c, WorldError::from(e)));
            }
        }
        Err(e) => sim.tick_faults.push((c, WorldError::Move(e))),
    }
    sim.announced_ground.insert((c, item));
    sim.walk_announced.push((c, item));
}

/// The ground update of one item.
struct GroundUpdateRun {
    item: UnitId,
}

impl MoveCall for GroundUpdateRun {
    type Out = Option<Result<Option<Vec<u8>>, MoveFatal>>;
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        let d = parts.desk(econ);
        let guid = d.guid_of(self.item);
        if !d.unit_exists(Owner::item(guid)) {
            return None;
        }
        let was_dropped = d.was_dropped(self.item);
        Some(sim_moves::ground_update_as(&d, guid, was_dropped))
    }
}

/// The host's answer to a room join's item mark
/// ([`d2_sim::wiring::action::JOIN_ITEM_MARK`]): the ground update of
/// an already announced item, queued to the
/// joining client where the join put it. "Already announced" is the
/// item's unit flag 0x10, read by the sim at the join (it sends no mark
/// while the flag is set), not whether this client got the item before:
/// an item first seen at a join gets 0x9C action 3 (action 2 when dropped
/// this tick). The client then counts as told, so the update pass does
/// not announce the item again.
pub fn join_marked<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    out: &mut dyn MessageSink,
    receiver: UnitId,
    guid: u32,
) {
    use d2_sim::units::UnitType;
    if !sim.announce_ground {
        return;
    }
    let Some(c) = sim.client_of(receiver) else {
        return;
    };
    let Some(item) = sim.game.lists.find_unit(UnitType::Item, guid) else {
        return;
    };
    let (game, events) = (&mut sim.game, &mut sim.events);
    let Some(Some(r)) = sim.world.moves(game, events, GroundUpdateRun { item }) else {
        return;
    };
    match r {
        Ok(Some(m)) => {
            if let Err(e) = out.queue(c, &m) {
                sim.tick_faults.push((c, WorldError::from(e)));
            }
            sim.announced_ground.insert((c, item));
        }
        Ok(None) => {}
        Err(e) => sim.tick_faults.push((c, WorldError::Move(e))),
    }
}

/// One player's item messages (the update-list pass, 0x47, 0x48) at the
/// player's place in the tick's queue walk.
struct PlayerItemsRun {
    own: UnitId,
    player: UnitId,
}

impl MoveCall for PlayerItemsRun {
    type Out = Result<Vec<Vec<u8>>, MoveFatal>;
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        let mut d = parts.desk(econ);
        let (own, guid) = (d.guid_of(self.own), d.guid_of(self.player));
        sim_moves::player_update(&mut d, own, guid)
    }
}

/// The item break of the inventory model: flag 0x100, the owner's
/// update list and refresh.
struct BreakRun {
    item: UnitId,
}

impl MoveCall for BreakRun {
    type Out = ();
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        let mut d = parts.desk(econ);
        d.break_item(self.item);
    }
}

/// The host's answer to an item-break mark
/// ([`d2_sim::wiring::action::ITEM_BREAK_MARK`]): the item with `guid`
/// breaks on the inventory model now.
pub fn break_marked<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    owner: UnitId,
    guid: u32,
) {
    use d2_sim::units::UnitType;
    let Some(item) = sim.game.lists.find_unit(UnitType::Item, guid) else {
        return;
    };
    let (game, events) = (&mut sim.game, &mut sim.events);
    let _ = sim.world.moves(game, events, BreakRun { item });
    // The break's last step, the client refresh with the cursor sound 9:
    // the owner's 0x2C goes out in its next unit update.
    let _ = d2_sim::units::sound::queue_sound(&mut sim.game, owner, 9, None);
}

/// The item refresh of a level-up on the inventory model: the item
/// relink, then the inventory pass (the owner refresh is the action
/// wiring's).
struct LevelUpRun {
    player: UnitId,
}

impl MoveCall for LevelUpRun {
    type Out = ();
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        use d2_sim::items::inventory::bookkeeping::{EquipWorld, USE_NO_LEVEL, USE_NO_QUANTITY};
        use d2_sim::items::moves::MovePending;
        use d2_sim::stats::lists::flag;
        let p = self.player;
        let mut d = parts.desk(econ);
        let Some(items) = d.state.inventories.get(&p).map(|inv| inv.items().to_vec()) else {
            return;
        };
        // The left and right mouse skills, saved before the relink.
        let saved = [d.mouse_skill(p, true), d.mouse_skill(p, false)];
        // Each item, in inventory order, whose stat list hangs under an
        // extended list: detached and attached again to the player, with
        // reset when the list is not dynamic (flag 0x40000000 clear).
        for i in items {
            let stats = &mut *d.econ.stats;
            let Some(l) = stats.unit_list(i) else {
                continue;
            };
            let attached = stats
                .parent(l)
                .is_some_and(|q| stats.flags(q) & flag::EXTENDED != 0);
            if !attached {
                continue;
            }
            let reset = stats.flags(l) & flag::DYNAMIC == 0;
            stats.detach(&mut *d.econ.hooks, l);
            stats.equip(&mut *d.econ.hooks, p, Some(l), false, reset);
        }
        // A saved skill the player still has and that is no longer on
        // its side is selected again, unless its use state is "no
        // quantity" or "no level".
        for (left, s) in [(true, saved[0]), (false, saved[1])] {
            let Some(s) = s.filter(|s| s.0 != 0) else {
                continue;
            };
            if !EquipWorld::has_skill(&d, p, s) || d.mouse_skill(p, left) == Some(s) {
                continue;
            }
            let st = d.use_state(p, s);
            if st != USE_NO_QUANTITY && st != USE_NO_LEVEL {
                d.select_skill(p, left, s);
            }
        }
        if let Some(o) = d.owner_of(p) {
            MovePending::inventory_pass(&mut d, o);
        }
    }
}

/// The host's answer to a level-up mark
/// ([`d2_sim::wiring::action::reaction::LEVEL_UP_ITEMS_MARK`]): the
/// player with `guid` gets the item relink and the inventory pass now.
pub fn level_up_marked<D: EventDispatch, W: WorldHost<D>>(sim: &mut SimGame<D, W>, guid: u32) {
    use d2_sim::units::UnitType;
    let Some(player) = sim.game.lists.find_unit(UnitType::Player, guid) else {
        return;
    };
    let (game, events) = (&mut sim.game, &mut sim.events);
    let _ = sim.world.moves(game, events, LevelUpRun { player });
}

struct ExhaustStackRun {
    player: UnitId,
    item: u32,
}

impl MoveCall for ExhaustStackRun {
    type Out = Result<(), MoveFatal>;
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        let mut desk = parts.desk(econ);
        let owner = Owner::player(desk.guid_of(self.player));
        sim_moves::handlers::exhaust_stack(&mut desk, owner, self.item)
    }
}

/// Remove the equipped stack whose final Throw has created its missile.
pub fn exhaust_stack_marked<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    player: UnitId,
    item: u32,
) {
    let (game, events) = (&mut sim.game, &mut sim.events);
    let _ = sim
        .world
        .moves(game, events, ExhaustStackRun { player, item });
}

/// A key a locked chest or door took, used up from the player's inventory.
struct KeyUseRun {
    player: UnitId,
}

impl MoveCall for KeyUseRun {
    type Out = bool;
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        let mut desk = parts.desk(econ);
        let owner = Owner::player(desk.guid_of(self.player));
        sim_moves::handlers::use_key(&mut desk, owner)
    }
}

/// The host's answer to a key-use mark
/// ([`d2_sim::wiring::action::pending::KEY_USE_MARK`]) addressed to
/// `player`: one key leaves the player's first key stack.
pub fn key_use_marked<D: EventDispatch, W: WorldHost<D>>(sim: &mut SimGame<D, W>, player: UnitId) {
    let (game, events) = (&mut sim.game, &mut sim.events);
    let _ = sim.world.moves(game, events, KeyUseRun { player });
}

/// A restored inactive item's child put back into its parent as a
/// socket filler, then the parent's refresh.
struct SocketRestoreRun {
    parent: UnitId,
    child: UnitId,
}

impl MoveCall for SocketRestoreRun {
    type Out = Option<()>;
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        parts.desk(econ).link_filler(self.parent, self.child)
    }
}

/// The host's answer to a socket-restore mark
/// ([`d2_sim::wiring::action::pending::SOCKET_RESTORE_MARK`]) addressed to
/// `parent`: the item with `guid` is linked into it now.
pub fn socket_restore_marked<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    parent: UnitId,
    guid: u32,
) {
    use d2_sim::units::UnitType;
    let Some(child) = sim.game.lists.find_unit(UnitType::Item, guid) else {
        return;
    };
    let (game, events) = (&mut sim.game, &mut sim.events);
    let _ = sim
        .world
        .moves(game, events, SocketRestoreRun { parent, child });
}

/// One hireling's item messages (its update list, then 0x48) at its
/// place in the tick's queue walk.
struct MercItemsRun {
    own: UnitId,
    merc: UnitId,
}

impl MoveCall for MercItemsRun {
    type Out = Option<Result<Vec<Vec<u8>>, MoveFatal>>;
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        let mut d = parts.desk(econ);
        let (own, guid) = (d.guid_of(self.own), d.guid_of(self.merc));
        d.merc_items_update(own, guid)
    }
}

/// The host's answer to a hireling item-update mark
/// ([`d2_sim::wiring::action::MERC_ITEMS_MARK`]): its item messages to the
/// marked client when the hireling is that client's player's.
pub fn merc_marked<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    out: &mut dyn MessageSink,
    receiver: UnitId,
    guid: u32,
) {
    use d2_sim::units::UnitType;
    let Some(c) = sim.client_of(receiver) else {
        return;
    };
    let Some(merc) = sim.game.lists.find_unit(UnitType::Monster, guid) else {
        return;
    };
    let (game, events) = (&mut sim.game, &mut sim.events);
    let Some(Some(r)) = sim.world.moves(
        game,
        events,
        MercItemsRun {
            own: receiver,
            merc,
        },
    ) else {
        return;
    };
    match r {
        Ok(msgs) => {
            for m in msgs {
                if let Err(e) = out.queue(c, &m) {
                    sim.tick_faults.push((c, WorldError::from(e)));
                }
            }
        }
        Err(e) => sim.tick_faults.push((c, WorldError::Move(e))),
    }
}

/// The host's answer to a player-update mark of the tick's client pass
/// ([`d2_sim::wiring::action::PLAYER_ITEMS_MARK`] /
/// [`d2_sim::wiring::action::PLAYER_SOUND_MARK`]): the item messages or the
/// sound of the player with `guid` to the marked client now, so they stand
/// where the queue walk put the player. The pass skips what went out here.
pub fn player_marked<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    out: &mut dyn MessageSink,
    receiver: UnitId,
    guid: u32,
    sound: bool,
) {
    use d2_sim::units::UnitType;
    let Some(c) = sim.client_of(receiver) else {
        return;
    };
    let Some(player) = sim.game.lists.find_unit(UnitType::Player, guid) else {
        return;
    };
    if sound {
        if sim.walk_player_sound.contains(&(c, player)) {
            return;
        }
        if let Some(m) = sound_message(&sim.game, player, receiver) {
            if let Err(e) = out.queue(c, &m) {
                sim.tick_faults.push((c, WorldError::from(e)));
            }
        }
        sim.walk_player_sound.push((c, player));
        return;
    }
    if sim.walk_player_items.contains(&(c, player)) {
        return;
    }
    let (game, events) = (&mut sim.game, &mut sim.events);
    let Some(r) = sim.world.moves(
        game,
        events,
        PlayerItemsRun {
            own: receiver,
            player,
        },
    ) else {
        return;
    };
    match r {
        Ok(msgs) => {
            for m in msgs {
                if let Err(e) = out.queue(c, &m) {
                    sim.tick_faults.push((c, WorldError::from(e)));
                }
            }
        }
        Err(e) => sim.tick_faults.push((c, WorldError::Move(e))),
    }
    sim.walk_player_items.push((c, player));
}
