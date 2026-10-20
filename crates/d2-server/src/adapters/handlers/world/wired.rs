//! [`WiredWorld`]: the wired single-player host. It is the `WorldHost`
//! implementation behind the world handlers of this module: the NPC, vendor,
//! quest and cube systems on their `d2-sim` providers
//! (`d2_sim::wiring::interaction`: [`Desk`] for `NpcWorld +
//! NpcVendors`, [`VendorDesk`] for `VendorWorld`;
//! `d2_sim::wiring::economy`: [`EconomyQuests`] for `QuestWorld`, on the
//! same desk; `EconomyCube` for the cube, `handlers::items`), beside the
//! waypoints and skill handlers of [`ActionWorld`].
//!
//! One unit world: the economy ([`Economy`]) is built per call from the
//! action wiring's own unit records, stat lists, unit data and hooks
//! (`ActionSim::sys`), so the player, the NPCs, the store items, the
//! cube's items and the monsters' drops are the same units the rest of
//! the game sees, in the game's one item store (`ActionHooks::items`,
//! lent to the economy for the call).
//!
//! One inventory per unit: the inventory model of the item moves
//! ([`WiredWorld::inventory`], `d2_sim::wiring::inventory`) is also the
//! vendors' ([`InvVendors`]: ownership, cursor, placement, removal of the
//! player's items) and the cube's (item list, checks, placement,
//! removal), so an item placed by C→S 0x18 can be sold and cubed, and an
//! item bought or transmuted lands where the moves see it.
//!
//! One home per game field: the game seed and the creation fields
//! (difficulty, expansion, game type, ladder; the item format follows
//! from the expansion) live on the action wiring (`ActionHooks::game_seed`,
//! `ActionHooks::ai_info`, `UnitData::expansion`, written at game
//! creation by
//! [`super::ActionEvents::create_game`]); the economy's [`GameFields`]
//! are built from them for each call and the seed is written back
//! ([`WiredWorld::with_economy`]). The game's unique bits live there
//! too (`ActionHooks::uniques`): one store for the handlers' economy, the
//! lent quest parts and the object and monster drops of the action
//! wiring (`ActionHooks::object_drops`).
//!
//! One owner of the player's interaction (target GUID, target type,
//! active): the player's unit record
//! (`d2_sim::units::record::InteractInfo`), which the NPC, vendor,
//! quest, cube, inventory, waypoint and object wirings read and set.
//!
//! What `d2-sim` leaves to the host (player data, the item copy, the item
//! routines of the vendor system that have no body of their own, the
//! transport of the rests' messages) is the rest `R` ([`TradeRest`]); its
//! messages leave through [`Outbox`].

use d2_sim::game::Game;
use d2_sim::items::moves::MoveFatal;
use d2_sim::items::ItemTables;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::action::Pending;
use d2_sim::wiring::action::{ActionHooks, ActionSim, ObjectCase};
use d2_sim::wiring::economy::{
    quest_objects, Economy, EconomyQuests, GameFields, HostQuests, LoanedInventory, QuestInv,
    QuestInventory, QuestLoan, QuestRest,
};
use d2_sim::wiring::interaction::{
    Desk, InteractionError, InteractionState, NpcInv, NpcInventory, NpcRest, PlayerQuestsRef,
    VendorDesk, VendorRest,
};
use d2_sim::world::hirelings::life;
use d2_sim::world::npc::NpcControl;
use d2_sim::world::quests::{HostRequest, QuestControl};
use d2_sim::world::vendors::{price, store, trade, tx, GlobalLists, VendorTables, VendorWorld};
use d2_sim::world::waypoints::{
    ObjectFacts, PlayerFacts, RoomRect, WaypointRecords, WaypointWorld,
};

use super::super::items::moves::preview_skills::SKILL_SHRINE_STATE;
use super::super::items::moves::{take_sent as inv_take_sent, InvParts, MoveCall};
use super::super::items::{CubeCall, CubeParts, CubeQuests, InvVendors};
use super::super::player::{self, HostFacts, Outcome as PlayerOutcome, Run as PlayerRun};
use super::super::skills::{Call as SkillCall, Handled as SkillHandled, NoSkills, SkillHost};
use super::super::walk::{WalkCall, WalkResult};
use super::{
    ActionEvents, ActionWorld, NpcCall, Outbox, QuestCall, VendorCall, WaypointCall, WorldError,
    WorldFault, WorldHost,
};
use d2_sim::world::npc::NpcWorld;

/// The seams of the NPC and vendor wiring without a provider: the
/// interaction rests of `d2_sim::wiring::interaction` and
/// the outbox their messages go to (`QuestRest::send`, and
/// `VendorRest::send_transaction` as `d2_sim::world::npc::transaction`
/// bytes), in send order. Its `NpcRest` interaction calls are the
/// host's one owner of the player's interaction.
pub trait TradeRest: NpcRest + VendorRest + QuestRest + PlayerQuestsRef + Outbox {}

impl<R: NpcRest + VendorRest + QuestRest + PlayerQuestsRef + Outbox> TradeRest for R {}

/// The wired host of a game on `ActionSim` (or `WorldSim`): the action
/// systems ([`ActionWorld`], with the skill slot `S`), the economy's own
/// parts (item tables; the item store and the unique bits are the action
/// wiring's `ActionHooks::items`, `ActionHooks::uniques`), the cube's parts, the inventory
/// model, the quests, the NPC control block, the vendor tables and the
/// interaction state (one vendor record per NPC record, the NPCs'
/// interaction lists), and the rest.
pub struct WiredWorld<R, S = NoSkills> {
    /// Waypoints, arrivals, the skill slot and the handlers' faults.
    pub action: ActionWorld<S>,
    /// The item tables of the economy, cube and inventory rules.
    pub tables: ItemTables,
    /// The cube (`None`: 0x2A, 0x4F stay stubs).
    pub cube: Option<CubeParts>,
    /// The game's one inventory model and the item-move seams (`None`:
    /// the item-move ids stay stubs, `handlers::items::moves`; the
    /// vendors and the cube see empty inventories).
    pub inventory: Option<InvParts>,
    /// The quest control block (quest records and chains of the game).
    pub quests: QuestControl,
    /// The NPC control block.
    pub npc: NpcControl,
    /// The vendor tables (store contents, price columns).
    pub vendor_tables: VendorTables,
    /// The interaction state: vendor records, interaction lists, trade windows.
    pub state: InteractionState,
    /// The rest `R`: what `d2-sim` leaves to the host, and the message outbox.
    pub rest: R,
    /// Monster classes whose units get an interaction list
    /// (the game's monster init embeds one for an `interact` NPC;
    /// D2RUST's own, unverified: monster init does not call
    /// `InteractionState::add_npc` yet, so a host names the classes and
    /// each desk call registers their units, idempotently).
    pub interact_classes: Vec<u16>,
    /// Host milliseconds (`GetTickCount`), an input of store generation
    /// and refresh; the caller keeps it current.
    pub now: u32,
    /// What the inventory rules queued during vendor calls (receiving
    /// unit, bytes), sent after the rest's messages ([`WorldHost::take_sent`]).
    pub(super) inv_sent: Vec<(UnitId, Vec<u8>)>,
    /// The store items a purchase took, their 0x9C action 12 sent with
    /// the next tick's unit work.
    pub(super) taken_sent: Vec<(UnitId, Vec<u8>)>,
    /// The store items a trade open added to the NPC's trade inventory,
    /// their 0x9C action 11 sent by the next tick's client pass
    /// ([`WorldHost::take_client_pass_sent`]; they follow the NPC's 0x8A and
    /// 0x6D).
    pub(super) shown_sent: Vec<(UnitId, Vec<u8>)>,
    /// Fatal paths met inside a desk call whose result type cannot carry
    /// them (the store-item send, [`flush_shown`]), reported
    /// with the handler's own ([`WorldHost::take_faults`]).
    pub(super) late_faults: Vec<WorldError>,
    /// A loaded Iron Golem item made at the `kf` read and waiting for the
    /// join's re-cast to equip it: the player ->
    /// (the item unit, its save entry).
    pub(super) golem_loose:
        std::collections::BTreeMap<UnitId, (UnitId, d2_formats::d2s::ItemEntry)>,
    /// The messages the systems sent so far, in production order
    /// ([`Self::collect_sent`]).
    pub(super) outbox: Vec<(UnitId, Vec<u8>)>,
    /// Pick-ups waiting for the player's run to the item to end
    /// (player, item GUID, cursor flag; [`Self::item_arrivals`]).
    pub(super) item_queued: Vec<(UnitId, u32, bool)>,
    /// The 0x13 object walks waiting for the run to end (player, object
    /// GUID; [`Self::object_arrivals`]).
    pub(super) object_queued: Vec<(UnitId, u32)>,
    /// The 0x13 tile walks waiting for the run to end (player, tile
    /// GUID; the pending 0x13 `{5, GUID}`,
    /// [`Self::object_arrivals`]).
    pub(super) tile_queued: Vec<(UnitId, u32)>,
    /// Ground items picked up by a move call (player, item), whose quest
    /// hook ITEMPICKEDUP (event 4) runs at the end of the call
    /// ([`Self::run_item_picks`]; its messages lead the pick-up's own).
    pub(super) item_picks: Vec<(UnitId, UnitId)>,
    /// An approach arrival's 0x13 is running ([`WiredWorld::handler_work`]
    /// starts no approach for it).
    pub(super) arriving: bool,
    /// D2RUST's own, unverified (REC-244): item codes a new character gets
    /// after its charstats start items, one each, to the inventory (the
    /// play preview names the Horadric Cube, `box `, which charstats
    /// does not give). Empty: the charstats start items only.
    pub start_extra: Vec<[u8; 4]>,
}

impl<R, S> WiredWorld<R, S> {
    /// The unit facts of an item: the
    /// owner from the inventory model (`InvItem::owner_guid`, the owning
    /// player's GUID), the act from the item's room, or from its owner
    /// when it is in an inventory, and the position of an item on the
    /// ground from its static path. `None`: not an item, or no act.
    fn item_facts<D: ActionEvents>(
        &mut self,
        game: &Game,
        events: &mut D,
        unit: UnitId,
    ) -> Option<crate::adapters::UnitFacts> {
        use d2_sim::units::UnitType;
        let e = game.lists.unit(unit)?;
        if e.ty != UnitType::Item {
            return None;
        }
        let owner = self
            .inventory
            .as_ref()
            .and_then(|i| i.state.items.get(&unit))
            .map(|i| i.owner_guid)
            .filter(|&g| g != d2_sim::items::inventory::NO_GUID)
            .and_then(|g| game.lists.find_unit(UnitType::Player, g));
        let (x, y) = events.action().hooks().path_position(unit);
        let act = match e.room() {
            Some(r) => game.lists.room(r)?.act,
            None => game.lists.room(game.lists.unit(owner?)?.room()?)?.act,
        };
        Some(crate::adapters::UnitFacts {
            act,
            pos: crate::seams::Pos { x, y },
            owner,
        })
    }
    /// The vendor records at game creation from the NPC records and the
    /// global column lists (`GlobalLists::build`), under D2RUST's store
    /// roles; a mod game's server (MS15) builds them with
    /// `InteractionState::new_for` and `GlobalLists::build_for`. The creation fields are the
    /// action wiring's ([`super::ActionEvents::create_game`]).
    pub fn new(
        action: ActionWorld<S>,
        tables: ItemTables,
        quests: QuestControl,
        npc: NpcControl,
        vendor_tables: VendorTables,
        rest: R,
        now: u32,
    ) -> Self {
        let state = InteractionState::new(&npc, &GlobalLists::build(&vendor_tables));
        Self {
            action,
            tables,
            cube: None,
            inventory: None,
            quests,
            npc,
            vendor_tables,
            state,
            rest,
            interact_classes: Vec::new(),
            now,
            inv_sent: Vec::new(),
            taken_sent: Vec::new(),
            shown_sent: Vec::new(),
            late_faults: Vec::new(),
            golem_loose: std::collections::BTreeMap::new(),
            outbox: Vec::new(),
            item_queued: Vec::new(),
            object_queued: Vec::new(),
            tile_queued: Vec::new(),
            item_picks: Vec::new(),
            arriving: false,
            start_extra: Vec::new(),
        }
    }

    /// Runs `f` on the economy over the action wiring's unit side
    /// (units, stat lists, unit data, hooks, the game's item store, lent
    /// out of the hooks for the call) and this world's item parts, with
    /// the game fields built from their home (game seed, creation fields)
    /// and the seed, the store and the unique bits written back after the
    /// call. Item creation outside a handler (a fixture) goes through it;
    /// the inventory model is in [`Parts::inventory`] (`InvParts::desk`
    /// on the same economy).
    pub fn with_economy<D: ActionEvents, T>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        f: impl FnOnce(&mut Economy<'_, ActionHooks<D::X>>, &mut Parts<'_, R>) -> T,
    ) -> T {
        // With the monster state lent: a monster the call creates (the
        // hired mercenary) gets its type init.
        events.lend_world(|a| self.with_economy_on(game, a, f))
    }

    fn with_economy_on<X: Pending, T>(
        &mut self,
        game: &mut Game,
        a: &mut ActionSim<X>,
        f: impl FnOnce(&mut Economy<'_, ActionHooks<X>>, &mut Parts<'_, R>) -> T,
    ) -> T {
        let s = &mut a.sys;
        let mut fields = GameFields::from_action(
            s.hooks.game_seed,
            &s.hooks.ai_info,
            s.data.expansion,
            std::mem::take(&mut s.hooks.uniques),
        );
        let mut items = std::mem::take(&mut s.hooks.items);
        let out = {
            let mut econ = Economy {
                game,
                units: &mut s.units,
                stats: &mut s.stats,
                data: &s.data,
                hooks: &mut s.hooks,
                fields: &mut fields,
                tables: &self.tables,
                items: &mut items,
            };
            let mut parts = Parts {
                quests: &mut self.quests,
                npc: &mut self.npc,
                vendor_tables: &self.vendor_tables,
                state: &mut self.state,
                cube: self.cube.as_mut(),
                inventory: self.inventory.as_mut(),
                rest: &mut self.rest,
                now: self.now,
            };
            f(&mut econ, &mut parts)
        };
        s.hooks.items = items;
        s.hooks.game_seed = fields.seed;
        s.hooks.uniques = fields.uniques;
        out
    }

    /// Runs `f` on the desk over [`Self::with_economy`]'s economy and
    /// this world, with the NPC control block and the inventory model.
    pub(super) fn desk<D: ActionEvents, T>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        f: impl FnOnce(
            &mut Desk<'_, '_, ActionHooks<D::X>, R>,
            &mut NpcControl,
            Option<&mut InvParts>,
        ) -> T,
    ) -> T {
        self.desk_with(game, events, false, f)
    }

    /// [`Self::desk`]; with `lend` the inventory model is lent to the
    /// desk for the NPC item services ([`NpcInv`], `Desk::inv`) and `f`
    /// gets none.
    pub(super) fn desk_with<D: ActionEvents, T>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        lend: bool,
        f: impl FnOnce(
            &mut Desk<'_, '_, ActionHooks<D::X>, R>,
            &mut NpcControl,
            Option<&mut InvParts>,
        ) -> T,
    ) -> T {
        let classes = self.interact_classes.clone();
        self.with_economy(game, events, |econ, p| {
            for u in econ.game.lists.units_of_type(UnitType::Monster) {
                let class = econ.units.get(u).map(|r| r.class);
                if class.is_some_and(|c| classes.iter().any(|&k| u32::from(k) == c)) {
                    p.state.add_npc(u);
                }
            }
            let mut lent = if lend { p.inventory.take() } else { None }.map(|v| {
                let InvParts {
                    tables,
                    state,
                    rest,
                } = v;
                NpcInv {
                    tables: &*tables,
                    state,
                    rest: rest.as_mut(),
                }
            });
            let mut desk = Desk {
                econ,
                quests: &mut *p.quests,
                vendor_tables: p.vendor_tables,
                state: &mut *p.state,
                rest: &mut *p.rest,
                now: p.now,
                inv: lent.as_mut().map(|v| v as &mut dyn NpcInventory<_>),
            };
            f(&mut desk, &mut *p.npc, p.inventory.as_deref_mut())
        })
    }
}

mod quest_events;

/// 0x9C action of a store item a purchase took from the NPC's grid (sent
/// with the next tick's unit work).
const STORE_TAKEN_ACTION: u8 = 12;

/// The store items a trade open added to the NPC's trade inventory, as
/// S→C 0x9C action 11 to the opening player, one per item in add order
/// Needs the inventory
/// model (the item bit stream); without it the items stay unsent.
/// A failed store-item send is a fatal fault of the game: the sends stop there and the fault is
/// returned with what was sent before it.
fn flush_shown<X: Pending, R: TradeRest>(
    desk: &mut Desk<'_, '_, ActionHooks<X>, R>,
    inv: Option<&mut InvParts>,
) -> (Vec<(UnitId, Vec<u8>)>, Option<MoveFatal>) {
    let (Some(parts), Some(player)) = (inv, desk.state.shown_player) else {
        return (Vec::new(), None);
    };
    let items = std::mem::take(&mut desk.state.shown);
    // D2RUST's own, unverified: the buy price of each shown item, for the
    // preview client (`VendorRest::store_price`).
    let (tables, class) = (desk.vendor_tables, desk.state.shown_class);
    // A gamble window shows the gamble price.
    let kind = if desk.state.shown_gamble {
        tx::GAMBLE
    } else {
        tx::BUY
    };
    let mut prices: Vec<(u32, u32)> = {
        let v = desk.vendors(None);
        let ctx = trade::price_ctx(tables, &v, player, class);
        items
            .iter()
            .filter_map(|&i| {
                let it = v.price_item(i)?;
                let c = price::cost(tables, &ctx, Some(&it), kind).ok()?;
                Some((v.guid(i), u32::try_from(c).ok()?))
            })
            .collect()
    };
    // D2RUST's own, unverified: the repair-all total of a repairer (the
    // game's store roles), under item GUID 0 (the repair-all message's item).
    let w = InvVendors::new(desk.vendors(None), Some(&mut *parts));
    if d2_sim::world::vendors::StoreRoles::of(w.rules(), class).repairs {
        let ctx = trade::price_ctx(tables, &w, player, class);
        if let Ok((total, _)) = trade::repair_all_quote(tables, &ctx, &w, player) {
            prices.push((0, u32::try_from(total).unwrap_or(0)));
        }
    }
    for (guid, price) in prices {
        desk.rest.store_price(player, guid, price);
    }
    let mut d = parts.desk(&mut *desk.econ);
    let mut fault = None;
    for item in items {
        // The store stream (alt-code for an unidentified quality 4–9 gamble
        // item). The send never skips an item: no client player is fatal
        // 0x77B, an encoded item + 8 reaching 0xFD bytes 0x78C.
        if let Err(e) = d.send_store_item(player, item) {
            fault = Some(e);
            break;
        }
    }
    let sent = inv_take_sent(&mut d)
        .into_iter()
        .filter_map(|(u, b)| Some((u?, b)))
        .collect();
    (sent, fault)
}

/// The store items a purchase took out of the NPC's grid, as S→C 0x9C
/// action 12 to the trading player, in take order. Without the inventory model the items stay
/// unsent.
fn flush_taken<X: Pending, R: TradeRest>(
    desk: &mut Desk<'_, '_, ActionHooks<X>, R>,
    inv: Option<&mut InvParts>,
) -> Vec<(UnitId, Vec<u8>)> {
    let taken = std::mem::take(&mut desk.state.taken);
    // A freed unit is no longer shown in the store.
    desk.state.shown.retain(|u| !taken.contains(u));
    let (Some(parts), Some(player)) = (inv, desk.state.shown_player) else {
        return Vec::new();
    };
    let mut d = parts.desk(&mut *desk.econ);
    for item in taken {
        let _ = d.send_item_world(player, item, STORE_TAKEN_ACTION, 0);
        // The taken unit is gone from the unit list in the same tick.
        d.free(item);
    }
    inv_take_sent(&mut d)
        .into_iter()
        .filter_map(|(u, b)| Some((u?, b)))
        .collect()
}

impl<R: TradeRest, S> WiredWorld<R, S> {
    /// Closes whatever interaction `player` has open (a vendor's trade or
    /// store window, the stash, the cube, or a trade with another player)
    /// and returns the S→C messages the player's client expects for the
    /// close. A player trade is closed on both sides: the other player's
    /// interaction is reset and its client is queued the same close.
    ///
    /// A re-entry uses this before it replays the client's messages
    /// (contracts v12 3.7): a window open across a lost connection must not
    /// stay open in the game or be reopened by the replay. The cube's open
    /// message (0x77 0x15) is filtered from a replay by the caller; the
    /// client closes the cube locally.
    pub fn close_client_interactions<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
    ) -> Vec<Vec<u8>> {
        use d2_sim::world::cube::{trade_action, INTERACT_CUBE};
        use d2_sim::world::npc::talk;
        use d2_sim::world::stash::INTERACT_OBJECT;

        let mut closes = Vec::new();
        // The inventory model is lent to the desk: a dropped gamble list's
        // items leave the player's gamble node there too.
        self.desk_with(game, events, true, |desk, _ctl, _inv| {
            let Some((ty, guid)) = desk.econ.units.get(player).and_then(|r| r.interact.get())
            else {
                return;
            };
            let player_guid = desk.econ.units.get(player).map_or(u32::MAX, |r| r.guid);
            if let Some(r) = desk.econ.units.get_mut(player) {
                r.interact.reset();
            }
            match ty {
                ty if ty == UnitType::Monster as u8 => {
                    // The vendor's list entry goes; the last one takes the
                    // player's gamble list with it, as the chat close does.
                    let npc = desk.econ.game.lists.find_unit(UnitType::Monster, guid);
                    if let Some(npc) = npc {
                        let mut last = false;
                        if let Some(list) = desk.state.lists.get_mut(&npc) {
                            let talking = list.state(player).is_some_and(|s| s >= talk::CHATTING);
                            last = talking && list.nodes.iter().all(|n| n.0 == player);
                            list.nodes.retain(|n| n.0 != player);
                        }
                        if last {
                            d2_sim::world::npc::NpcVendors::drop_gamble_list(desk, player, npc);
                        }
                    }
                    // The client's trade/chat close (it closes the NPC dialog
                    // and a player-trade window; a shop panel is local).
                    closes.push(trade_action(0x0C).to_vec());
                }
                ty if ty == INTERACT_OBJECT => {
                    // The stash view (the waypoint's menu is local too).
                    closes.push(trade_action(0x11).to_vec());
                }
                ty if ty == UnitType::Player as u8 => {
                    // A player trade: the partner's side goes too, and its
                    // client is told to close.
                    let other = desk.econ.game.lists.find_unit(UnitType::Player, guid);
                    if let Some(other) = other {
                        let back = desk
                            .econ
                            .units
                            .get(other)
                            .and_then(|r| r.interact.get())
                            .is_some_and(|(t, g)| t == UnitType::Player as u8 && g == player_guid);
                        if back {
                            if let Some(r) = desk.econ.units.get_mut(other) {
                                r.interact.reset();
                            }
                        }
                        desk.rest.send(other, &trade_action(0x0C));
                    }
                    closes.push(trade_action(0x0C).to_vec());
                }
                ty if ty == INTERACT_CUBE => {
                    // The cube: the client closes its panel by itself.
                }
                _ => {}
            }
        });
        closes
    }

    /// Moves what the systems sent since the last call into the one
    /// outbox: the action wiring's sends, then what the inventory rules
    /// queued, then the rest's (NPC, vendor and quest messages). Called
    /// after each step that sends, so the outbox holds a tick's messages
    /// in production order; one system
    /// runs per step, and within a vendor call the inventory messages
    /// precede its 0x2A.
    pub(super) fn collect_sent<D: ActionEvents>(&mut self, events: &mut D)
    where
        D::X: Outbox,
    {
        let mut sent = events.action().hooks().x.take_sent();
        self.outbox.append(&mut sent);
        self.outbox.append(&mut self.inv_sent);
        self.outbox.extend(self.rest.take_sent());
    }

    /// Store items to show: queued for update ahead of the NPC room's
    /// current queue (the update list prepends), so the walk sends them before the unit
    /// that heads the queue now.
    fn queue_shown<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        sent: Vec<(UnitId, Vec<u8>)>,
    ) {
        if self.shown_sent.is_empty() && !sent.is_empty() {
            let npc = self.desk(game, events, |desk, _, _| desk.state.shown_npc);
            let anchor = npc
                .and_then(|n| game.lists.unit(n))
                .and_then(|u| u.room())
                .map(|r| (r, game.lists.update_first(r)));
            events.action().hooks().store_anchor = anchor;
        }
        self.shown_sent.extend(sent);
    }

    /// The vendor part of the town-leave refreshes the action wiring
    /// queued (`ActionHooks::town_leaves`): the refresh
    /// (`store::refresh_act`) of the left town's act on this host's vendor
    /// records, with `empty` as counted at the call. Run when the tick or
    /// handler that queued them returns.
    pub(super) fn town_leaves<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D) {
        let q = std::mem::take(&mut events.action().sys.hooks.town_leaves);
        if q.is_empty() {
            return;
        }
        let now = self.now;
        let sent = self.desk(game, events, |desk, ctl, inv| {
            let mut records = std::mem::take(&mut desk.state.vendors);
            let inner: VendorDesk<'_, '_, '_, _, _> = desk.vendors(Some(ctl));
            let mut w = InvVendors::new(inner, inv);
            for (_, from, empty) in q {
                let act = u16::try_from(from).ok().and_then(store::town_act);
                if let Some(act) = act {
                    store::refresh_act(&mut records, &mut w, act, empty, now);
                }
            }
            let sent = std::mem::take(&mut w.sent);
            drop(w);
            desk.state.vendors = records;
            sent
        });
        self.inv_sent.extend(sent);
    }
}

/// The wired host's inventory model as a quest loan's ([`QuestLoan`]).
impl LoanedInventory for InvParts {
    fn lend<X: Pending, T>(
        &mut self,
        f: impl FnOnce(&mut dyn QuestInventory<ActionHooks<X>>) -> T,
    ) -> T {
        let mut q = QuestInv::new(&self.tables, &mut self.state, self.rest.as_mut());
        let out = f(&mut q);
        let errors = q.errors;
        self.state.errors.extend(
            errors
                .into_iter()
                .map(d2_sim::wiring::inventory::InvError::Economy),
        );
        out
    }
}

/// A quest call on the desk's economy and rest ([`HostQuests`]: the
/// [`EconomyQuests`] calls with the object, level, interaction and
/// identify calls answered by the action wiring and the NPC rest, the
/// NPCs' interaction lists of the interaction state and, when the host
/// has one, the inventory model, [`QuestInv`]: the reward and the cube
/// close): the mercenary rewards an Act I quest grants are queued
/// during the call with the sends that follow them
/// ([`d2_sim::wiring::economy::QuestDeferred`]) and run on the NPC
/// control block right after it, then the queued sends.
/// A reward's NPC error, and an item
/// creation error of the reward, go to the interaction state's errors.
/// Also returned: what the inventory model sent during the call
/// (receiving unit, bytes), for [`WorldHost::take_sent`]'s inventory part.
///
/// TODO: the order of a reward's inventory messages against the quest
/// messages of the same call is not settled; they follow the rest's, as
/// the vendor calls' do.
fn quest_call<X: Pending, R: TradeRest, T>(
    desk: &mut Desk<'_, '_, ActionHooks<X>, R>,
    ctl: &mut NpcControl,
    mut inv: Option<&mut InvParts>,
    f: impl FnOnce(&mut QuestControl, &mut HostQuests<'_, '_, X, R>) -> T,
) -> (T, Vec<(UnitId, Vec<u8>)>) {
    let mut deferred = Vec::new();
    let mut errors = Vec::new();
    let out = {
        let mut lent = inv
            .as_deref_mut()
            .map(|p| QuestInv::new(&p.tables, &mut p.state, p.rest.as_mut()));
        let out = {
            let mut inner = EconomyQuests::new(&mut *desk.econ, &mut *desk.rest);
            inner.deferred = Some(&mut deferred);
            let mut w = HostQuests::new(inner);
            w.chats = Some(&mut desk.state.lists);
            w.inventory = lent
                .as_mut()
                .map(|q| q as &mut dyn QuestInventory<ActionHooks<X>>);
            f(&mut *desk.quests, &mut w)
        };
        if let Some(q) = lent {
            errors = q.errors;
        }
        out
    };
    for e in errors {
        desk.state.errors.push(InteractionError::Economy(e));
    }
    let sent = match inv {
        Some(p) => {
            let mut d = p.desk(&mut *desk.econ);
            inv_take_sent(&mut d)
                .into_iter()
                .filter_map(|(u, b)| Some((u?, b)))
                .collect()
        }
        None => Vec::new(),
    };
    for d in deferred {
        let Some((p, class)) = d.run(&mut *desk.rest) else {
            continue;
        };
        if let Err(e) = ctl.quest_mercenary(desk, p, class) {
            desk.state.errors.push(InteractionError::Npc(e));
        }
    }
    (out, sent)
}

/// The object module's queued quest routes
/// (`ActionSim::route_quest_objects`) run on the quest control
/// (`d2_sim::wiring::economy::quest_objects`) until none is left; the
/// routes no quest rule handles go to the action wiring's
/// `Pending::object_route`, as without the queue. Returns what the
/// inventory model sent ([`quest_call`]).
fn quest_objects<X: Pending, R: TradeRest>(
    desk: &mut Desk<'_, '_, ActionHooks<X>, R>,
    ctl: &mut NpcControl,
    mut inv: Option<&mut InvParts>,
) -> Vec<(UnitId, Vec<u8>)> {
    let mut sent = Vec::new();
    loop {
        let calls = desk
            .econ
            .hooks
            .objects
            .as_mut()
            .map(|s| s.take_quest_calls())
            .unwrap_or_default();
        if calls.is_empty() {
            return sent;
        }
        let (back, s) = quest_call(desk, ctl, inv.as_deref_mut(), |q, w| {
            quest_objects::run_all(q, w, calls)
        });
        sent.extend(s);
        for r in back {
            desk.econ.hooks.x.object_route(desk.econ.game, r);
        }
    }
}

impl<R: TradeRest + Default + 'static, S> WiredWorld<R, S> {
    /// Runs `f` with this world's quest parts (the quest control, the
    /// rest, the item tables) lent to the action hooks
    /// ([`QuestLoan`], `ActionHooks::quest_host`), so a quest init,
    /// operate or object event 7 the object module hands back during `f`
    /// runs at once, inside its allocation, dispatch or timer event. The
    /// parts come back after `f`; `f` must not use them through `self`
    /// (it gets only the action world). Hooks that already hold a quest
    /// host keep it.
    pub fn lend_quests<D: ActionEvents, T>(
        &mut self,
        events: &mut D,
        f: impl FnOnce(&mut ActionWorld<S>, &mut D) -> T,
    ) -> T {
        if events.action().sys.hooks.quest_host.is_some() {
            return f(&mut self.action, events);
        }
        let empty = self.quests.emptied();
        let loan = QuestLoan {
            quests: std::mem::replace(&mut self.quests, empty),
            rest: std::mem::take(&mut self.rest),
            tables: std::mem::take(&mut self.tables),
            // The inventory model goes with the loan: a quest object's
            // operate reads and removes the player's items
            // (q-a4-quest-items).
            inv: self.inventory.take(),
            // The NPCs' interaction lists: the AI's interaction tests
            // read them during the tick.
            chats: std::mem::take(&mut self.state.lists),
        };
        events.action().sys.hooks.quest_host = Some(Box::new(loan));
        let out = f(&mut self.action, events);
        let back = events
            .action()
            .sys
            .hooks
            .quest_host
            .take()
            .map(|h| h.into_any().downcast::<QuestLoan<R, InvParts>>());
        match back {
            Some(Ok(l)) => {
                let l = *l;
                self.quests = l.quests;
                self.rest = l.rest;
                self.tables = l.tables;
                self.inventory = l.inv;
                self.state.lists = l.chats;
            }
            // `f` took the loan out of the hooks or put another one in:
            // the game's quest state is gone (API misuse, fatal).
            _ => panic!("WiredWorld::lend_quests: the lent quest parts did not come back"),
        }
        out
    }
}

impl<R: TradeRest + Default + 'static, S> WiredWorld<R, S> {
    /// The host's unit work of one tick, run at the end of tick step 4
    /// (after the timer queue, before the client pass): the game runs each
    /// of these inside a timer event of step 4 (movement, the kill, the
    /// death mode), so their messages go out with the same tick's client
    /// pass. In
    /// order: the approach arrivals (a run that stopped in this tick's
    /// step 4, so after frame += 1), the item pick-up arrivals, the
    /// players' death starts, the corpses' items, the
    /// pet deaths, the approach runs requested, the hireling calls (NPC
    /// act changes included) and the pet follows on the lent monster
    /// world, the pet sweep and the hireling facts. D2RUST's own, unverified:
    /// the order within this block; that it runs after the whole timer
    /// queue rather than inside the event that raised it (the host's
    /// parts are not lent to the timer events).
    ///
    /// Each step's sends join the outbox before the next one runs
    /// ([`WiredWorld::collect_sent`]).
    pub(super) fn timer_step_work<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D)
    where
        Self: WorldHost<D>,
        D::X: Outbox,
    {
        self.inv_sent.append(&mut self.taken_sent);
        self.collect_sent(events);
        {
            let _span = crate::perf::span("world.arrivals");
            self.arrivals(game, events);
        }
        self.collect_sent(events);
        {
            let _span = crate::perf::span("world.item_arrivals");
            self.item_arrivals(game, events);
        }
        self.collect_sent(events);
        {
            let _span = crate::perf::span("world.object_arrivals");
            self.object_arrivals(game, events);
        }
        self.collect_sent(events);
        events.action().player_deaths(game);
        self.collect_sent(events);
        {
            let _span = crate::perf::span("world.corpse_fill");
            self.corpse_fill(game, events);
        }
        self.collect_sent(events);
        {
            let _span = crate::perf::span("world.pet_deaths");
            self.pet_deaths(game, events);
        }
        self.collect_sent(events);
        {
            let _span = crate::perf::span("world.approaches");
            self.approaches(game, events);
        }
        self.collect_sent(events);
        // The monster init of a hireling the calls allocate needs the
        // lent world, as in the session work.
        events.lend_world(|a| {
            self.hireling_calls(game, a);
            self.collect_sent(a);
            self.pet_follows(game, a);
            self.collect_sent(a);
        });
        {
            let _span = crate::perf::span("world.drive_hirelings");
            self.drive_hirelings(game, events);
        }
        self.collect_sent(events);
    }

    /// The unit work a C→S handler raised, run when the handler returns
    /// (inside the drain): pet deaths, the approach runs requested, the
    /// hireling calls (NPC travel's act change), the pet follows. The game
    /// runs these inside the handler itself.
    /// An approach arrival's own 0x13 starts no new approach
    /// (`WiredWorld::arrivals`). A poke directive (it calls the game's
    /// function, which runs its unit work inline) runs it when the
    /// directive returns, so a `warp`'s pet follow lands before the next
    /// tick's movement.
    pub fn handler_work<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D)
    where
        D::X: Outbox,
    {
        self.pet_deaths(game, events);
        if self.arriving {
            self.state.approaches.clear();
        } else {
            self.approaches(game, events);
        }
        self.hireling_calls(game, events);
        self.pet_follows(game, events);
        self.run_cube_staff(game, events);
    }

    /// The pet follows the placements queued
    /// (`ActionHooks::pet_follows`, on from the first frame): the hireling
    /// follow on the hireling list ([`life::follow`]; the other pet types
    /// have no list in `d2-sim` yet). A game without hireling tables has
    /// no hireling: the queue is dropped.
    ///
    /// The game runs the follow inside the placement: the messages the host held when
    /// the follow was queued (`ActionHooks::pet_follow_mark`) go out
    /// first, then the follow's, then the rest of the handler's.
    pub fn pet_follows<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D)
    where
        D::X: Outbox,
    {
        let hooks = events.action().hooks();
        let q = hooks
            .pet_follows
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        let mark = hooks.pet_follow_mark.take();
        let mut tail = Vec::new();
        if let (Some(m), false) = (mark, q.is_empty()) {
            let mut sent = hooks.x.take_sent();
            tail = sent.split_off(m.min(sent.len()));
            self.outbox.append(&mut sent);
        }
        // The summoned pet types have their lists
        // on the action hooks; the hireling's is the desk's below.
        for &p in &q {
            events.action().summon_follow(game, p);
        }
        if !q.is_empty() && self.state.hireling_tables.is_some() {
            self.desk(game, events, |desk, _, _| {
                desk.with_hirelings(|w, t, st| {
                    for p in q {
                        life::follow(w, t, st, p);
                    }
                })
            });
        }
        if !tail.is_empty() {
            let mut sent = events.action().hooks().x.take_sent();
            self.outbox.append(&mut sent);
            self.outbox.append(&mut self.inv_sent);
            self.outbox.extend(self.rest.take_sent());
            self.outbox.append(&mut tail);
        }
    }

    /// The items of the corpses the deaths created
    /// (`ActionHooks::death.loot`): the cursor
    /// and body items move from the player to the corpse. A host without
    /// the inventory model drops the queue.
    pub fn corpse_fill<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D)
    where
        Self: WorldHost<D>,
    {
        let hooks = &mut events.action().sys;
        let q = std::mem::take(&mut hooks.hooks.death.loot);
        let gold = std::mem::take(&mut hooks.hooks.death.gold_drops);
        if (q.is_empty() && gold.is_empty()) || self.inventory.is_none() {
            return;
        }
        let gold: Vec<_> = gold
            .into_iter()
            .filter(|&(_, a)| a > 0)
            .filter_map(|(p, a)| {
                Some((
                    d2_sim::items::moves::Owner::player(hooks.units.get(p)?.guid),
                    a,
                ))
            })
            .collect();
        let pairs: Vec<_> = q
            .into_iter()
            .filter_map(|(p, c)| {
                let (pr, cr) = (hooks.units.get(p)?, hooks.units.get(c)?);
                Some((
                    d2_sim::items::moves::Owner::player(pr.guid),
                    d2_sim::items::moves::Owner::player(cr.guid),
                    cr.class,
                ))
            })
            .collect();
        let Some((faults, sent)) = self.moves(
            game,
            events,
            super::super::items::moves::CorpseFillRun { pairs, gold },
        ) else {
            return;
        };
        debug_assert!(faults.is_empty(), "corpse fill: {faults:?}");
        for (unit, bytes) in sent {
            if let Some(u) = unit {
                self.inv_sent.push((u, bytes));
            }
        }
    }

    /// The hireling deaths the kill queued (`ActionHooks::pet_deaths`, on
    /// from the first frame): the hireling death rule
    /// ([`life::on_kill`] with flag 1) for each killed monster with a
    /// player owner and a hireling node; then the owners' deaths the
    /// player mode-17 start queued (`ActionHooks::owner_deaths`): the
    /// owner-death rule ([`life::player_death`], every
    /// game type). Without hireling tables the game has no hireling: the
    /// queues are dropped.
    ///
    /// TODO: the game runs the hireling death rule inside the kill,
    /// before the killer bookkeeping and the death mode, and the
    /// owner-death rule inside the mode-17 start right
    /// after the corpse creation; here both run when the handler or tick
    /// that queued them returns (the hireling state is this host's, not
    /// the action wiring's), so their 0x9B / 0x7A follow the call's other
    /// messages, and a kill and an owner death queued in one call run
    /// kills first.
    pub fn pet_deaths<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D) {
        let hooks = &mut events.action().sys.hooks;
        let q = hooks
            .pet_deaths
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        let owners = hooks
            .owner_deaths
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        if (q.is_empty() && owners.is_empty()) || self.state.hireling_tables.is_none() {
            return;
        }
        self.desk(game, events, |desk, _, _| {
            desk.with_hirelings(|w, _, st| {
                for m in q {
                    life::on_kill(w, st, m, true);
                }
                for p in owners {
                    life::player_death(w, st, p);
                }
            })
        });
    }
}

/// The host's quest control lent to one cube call ([`CubeQuests`]): the
/// Cow portal runs on [`HostQuests`] over the call's economy, as the
/// quest calls of [`quest_call`] do.
struct CubeQuestHost<'q, R> {
    quests: &'q mut QuestControl,
    rest: &'q mut R,
}

impl<X: Pending, R: TradeRest> CubeQuests<ActionHooks<X>> for CubeQuestHost<'_, R> {
    fn cow_portal(&mut self, econ: &mut Economy<'_, ActionHooks<X>>, player: UnitId) -> bool {
        let mut w = HostQuests::new(EconomyQuests::new(econ, &mut *self.rest));
        d2_sim::world::quests::cow_portal(self.quests, &mut w, player)
    }
}

/// The parts of a [`WiredWorld`] beside the economy, borrowed for one
/// [`WiredWorld::with_economy`] call.
pub struct Parts<'p, R> {
    /// The quest control block.
    pub quests: &'p mut QuestControl,
    /// The NPC control block.
    pub npc: &'p mut NpcControl,
    /// The vendor tables.
    pub vendor_tables: &'p VendorTables,
    /// The interaction state.
    pub state: &'p mut InteractionState,
    /// The cube parts, when the host has a cube.
    pub cube: Option<&'p mut CubeParts>,
    /// The inventory model (`InvParts::desk` on the call's economy).
    pub inventory: Option<&'p mut InvParts>,
    /// The rest `R`.
    pub rest: &'p mut R,
    /// Host milliseconds, as [`WiredWorld::now`].
    pub now: u32,
}

/// The action wiring's [`WaypointWorld`] with the difficulty of the
/// game's home (`ActionHooks::ai_info`); every other call goes to the
/// action wiring.
pub struct HostWaypoints<'w, W> {
    /// The action wiring's waypoint world.
    pub inner: &'w mut W,
    /// The game's difficulty, answered for [`WaypointWorld::difficulty`].
    pub difficulty: u8,
}

impl<W: WaypointWorld> WaypointWorld for HostWaypoints<'_, W> {
    fn frame(&self) -> i32 {
        self.inner.frame()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.inner.tick_rate()
    }
    fn difficulty(&self) -> u8 {
        self.difficulty
    }
    fn records(&mut self, player: UnitId) -> Option<&mut WaypointRecords> {
        self.inner.records(player)
    }
    fn object(&self, guid: u32) -> Option<(UnitId, ObjectFacts)> {
        self.inner.object(guid)
    }
    fn player(&self, player: UnitId) -> PlayerFacts {
        self.inner.player(player)
    }
    fn room_rect(&self, room: RoomId) -> RoomRect {
        self.inner.room_rect(room)
    }
    fn set_object_mode(&mut self, object: UnitId, mode: u8) {
        self.inner.set_object_mode(object, mode);
    }
    fn schedule_endanim(&mut self, object: UnitId, frame: i32) {
        self.inner.schedule_endanim(object, frame);
    }
    fn player_busy(&self, player: UnitId) -> bool {
        self.inner.player_busy(player)
    }
    fn set_interact(&mut self, player: UnitId, unit_type: u8, guid: u32) {
        self.inner.set_interact(player, unit_type, guid);
    }
    fn reset_interact(&mut self, player: UnitId) {
        self.inner.reset_interact(player);
    }
    fn interact_guid(&self, player: UnitId) -> Option<u32> {
        self.inner.interact_guid(player)
    }
    fn hostile_delay(&self, player: UnitId) -> bool {
        self.inner.hostile_delay(player)
    }
    fn attach_sound(&mut self, player: UnitId, event: u8) {
        self.inner.attach_sound(player, event);
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.inner.send(player, msg);
    }
    fn warp(&mut self, player: UnitId, level: u32, tile_code: u8) {
        self.inner.warp(player, level, tile_code);
    }
    fn spawn_room(&mut self, level: u32, tile_code: u8) -> Option<RoomId> {
        self.inner.spawn_room(level, tile_code)
    }
    fn set_player_mode_arrival(&mut self, player: UnitId) {
        self.inner.set_player_mode_arrival(player);
    }
}

/// A waypoint call on the action wiring's view, wrapped by
/// [`HostWaypoints`].
struct HostWaypointRun<C> {
    call: C,
    difficulty: u8,
}

impl<C: WaypointCall> WaypointCall for HostWaypointRun<C> {
    type Out = C::Out;
    fn call<W: WaypointWorld>(
        self,
        data: &d2_sim::world::waypoints::WaypointData,
        arrivals: &mut d2_sim::world::waypoints::ArrivalList,
        w: &mut W,
    ) -> C::Out {
        let mut hw = HostWaypoints {
            inner: w,
            difficulty: self.difficulty,
        };
        self.call.call(data, arrivals, &mut hw)
    }
}

impl<D: ActionEvents, R: TradeRest + Default + 'static, S: SkillHost<D>> WorldHost<D>
    for WiredWorld<R, S>
where
    D::X: Outbox,
{
    fn npc<C: NpcCall>(&mut self, game: &mut Game, events: &mut D, call: C) -> Option<C::Out> {
        // The player inventories are staged on the rest for the NPC
        // entries (Cain's identify, `inventory_entries`).
        self.desk(game, events, |desk, _, inv| {
            let players = desk.econ.game.lists.units_of_type(UnitType::Player);
            if let Some(p) = inv {
                let d = p.desk(&mut *desk.econ);
                for &pl in &players {
                    desk.rest.stage_inventory(pl, d.npc_entries(pl));
                }
            }
        });
        // The call runs with the inventory lent to the desk (the item
        // services: imbue, `Desk::inv`).
        let out = self.desk_with(game, events, true, |desk, ctl, _| {
            desk.state.defer_chat_end = true;
            let out = call.call(ctl, desk);
            desk.state.defer_chat_end = false;
            out
        });
        // The chat-close quest calls ran queued: now on the full quest
        // world (a quest's chat end may place an object, e.g. Tyrael's
        // last portal).
        let (_, sent) = self.desk(game, events, |desk, ctl, inv| {
            let ends = std::mem::take(&mut desk.state.chat_ends);
            quest_call(desk, ctl, inv, |q, w| {
                for (p, n) in ends {
                    q.npc_deactivate(w, p, n);
                }
            })
        });
        self.inv_sent.extend(sent);
        let (_, sent) = self.desk(game, events, |desk, _, mut inv| {
            let players = desk.econ.game.lists.units_of_type(UnitType::Player);
            // Cain's identify (C→S 0x34) on the inventory model.
            let done = desk.rest.take_identified();
            if let (false, Some(p)) = (done.is_empty(), inv.as_deref_mut()) {
                let mut d = p.desk(&mut *desk.econ);
                for item in done {
                    if let Some(&pl) = players.iter().find(|&&pl| d.state.holds(pl, item)) {
                        d.identify_unit(pl, item);
                    }
                }
            }
            ((), flush_shown(desk, inv))
        });
        let (sent, fault) = sent;
        self.late_faults.extend(fault.map(WorldError::from));
        self.queue_shown(game, events, sent);
        self.handler_work(game, events);
        Some(out)
    }

    /// The vendor records are lent out of the interaction state for the
    /// call (the module holds them while it calls the world, as
    /// `VendorDesk`'s own entry points do); the world is [`VendorDesk`]
    /// with the NPC control block (`NpcLink`), its player inventories
    /// answered by the inventory model ([`InvVendors`]).
    fn vendors<C: VendorCall>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        call: C,
    ) -> Option<C::Out> {
        let (out, sent) = self.desk(game, events, |desk, ctl, inv| {
            let mut records = std::mem::take(&mut desk.state.vendors);
            let tables = desk.vendor_tables;
            let shown = desk.state.shown_npc;
            let mut inner: VendorDesk<'_, '_, '_, _, _> = desk.vendors(Some(ctl));
            // The NPC whose trade is open: a sold copy is placed in its
            // store grid.
            inner.npc = shown;
            let mut inv = inv;
            let mut w = InvVendors::new(inner, inv.as_deref_mut());
            let out = call.call(tables, &mut records, &mut w);
            let sent = std::mem::take(&mut w.sent);
            drop(w);
            desk.state.vendors = records;
            let taken = flush_taken(desk, inv.as_deref_mut());
            // A sold copy placed in the open store (marked, added to the
            // trade inventory) is shown like the open's items: 0x9C
            // action 11 by the next client pass.
            let shown = if desk.state.shown.is_empty() {
                (Vec::new(), None)
            } else {
                flush_shown(desk, inv)
            };
            ((out, taken, shown), sent)
        });
        self.inv_sent.extend(sent);
        self.taken_sent.extend(out.1);
        let (shown, fault) = out.2;
        self.late_faults.extend(fault.map(WorldError::from));
        self.queue_shown(game, events, shown);
        let out = out.0;
        Some(out)
    }

    /// The action wiring's waypoints, with the game's difficulty
    /// (`HostWaypoints`).
    fn waypoints<C: WaypointCall>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        call: C,
    ) -> Option<C::Out> {
        let difficulty = events.action().hooks().ai_info.difficulty;
        let run = HostWaypointRun { call, difficulty };
        // The act change of a travel builds the new act and its objects:
        // a quest object's init runs inside its allocation on the lent
        // quest parts (Lut Gholein's start Jerhyn), as in `objects`.
        let out = self.lend_quests(events, |a, ev| WorldHost::<D>::waypoints(a, game, ev, run));
        let sent = self.desk(game, events, quest_objects);
        self.inv_sent.extend(sent);
        self.pet_deaths(game, events);
        self.hireling_calls(game, events);
        self.pet_follows(game, events);
        out
    }

    /// The action wiring's 0x13 object case ([`ActionWorld`]); a quest
    /// operate it queued runs right after the dispatch, on this world's
    /// quest control ([`quest_objects`]: nothing follows the operate in
    /// the handler).
    fn objects(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
        guid: u32,
    ) -> Option<ObjectCase> {
        let out = self.lend_quests(events, |a, ev| {
            WorldHost::<D>::objects(a, game, ev, player, guid)
        });
        let sent = self.desk(game, events, quest_objects);
        self.inv_sent.extend(sent);
        let sent = self.take_inventory_sent(game, events);
        self.inv_sent.extend(sent);
        out
    }

    /// The 0x13 tile case on the action wiring.
    fn warp_tile(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
        guid: u32,
    ) -> Option<u32> {
        let out = WorldHost::<D>::warp_tile(&mut self.action, game, events, player, guid);
        // A run to the tile started (5..=50): the 0x13 comes again on the
        // player's stop.
        let walks = events
            .action()
            .with(game, |_, v| std::mem::take(&mut v.h.tile_walks));
        for (p, g) in walks {
            self.drop_queued(p);
            self.item_queued.retain(|q| q.0 != p);
            self.object_queued.retain(|q| q.0 != p);
            self.tile_queued.retain(|q| q.0 != p);
            self.tile_queued.push((p, g));
        }
        out
    }

    /// The run to a ground item.
    fn item_walk(&mut self, game: &mut Game, events: &mut D, walk: (UnitId, UnitId, bool)) {
        self.start_item_walk(game, events, walk);
    }
    fn object_walk(&mut self, game: &mut Game, events: &mut D, walk: (UnitId, UnitId)) {
        self.start_object_walk(game, events, walk, false);
    }

    /// The tick with this world's quest parts lent to the action hooks
    /// ([`WiredWorld::lend_quests`]): quest object inits run inside their
    /// allocation and object event 7 inside its timer event, in the tick
    /// that runs them.
    /// The host's unit work (`WiredWorld::timer_step_work`) runs at the
    /// end of step 4, after the timer queue and before the client pass,
    /// so its messages reach the same
    /// tick's client pass.
    fn run_tick_through_timers(&mut self, game: &mut Game, events: &mut D)
    where
        D: d2_sim::tick::EventDispatch + d2_sim::tick::TickHooks,
    {
        self.lend_quests(events, |_, ev| {
            {
                let _span = crate::perf::span("sim.rooms");
                d2_sim::tick::tick_through_rooms(game, ev);
            }
            {
                let _span = crate::perf::span("sim.timers");
                d2_sim::tick::run_timer_events(game, ev);
            }
        });
        self.collect_sent(events);
        {
            let _span = crate::perf::span("world.timer_step_work");
            self.timer_step_work(game, events);
        }
        {
            let _span = crate::perf::span("world.kill_events");
            self.run_kill_events(game, events);
        }
        self.collect_sent(events);
    }

    /// Steps 5–11 with the quest parts lent, then the client pass's
    /// inventory messages.
    fn run_tick_from_client_pass(&mut self, game: &mut Game, events: &mut D)
    where
        D: d2_sim::tick::EventDispatch + d2_sim::tick::TickHooks,
    {
        {
            let _span = crate::perf::span("sim.client_pass");
            self.lend_quests(events, |_, ev| {
                d2_sim::tick::tick_from_client_pass(game, ev)
            });
        }
        let sent = self.take_inventory_sent(game, events);
        self.inv_sent.extend(sent);
        self.collect_sent(events);
    }

    /// The quest routes queued outside a lent call (a quest call's own
    /// allocations, [`quest_objects`]), before the tick's sends are taken;
    /// then the quest events queued with no quest control lent
    /// (`wired/quest_events.rs`).
    fn session_work(&mut self, game: &mut Game, events: &mut D) {
        // The monster init of a hireling the calls allocate needs the
        // lent world.
        events.lend_world(|a| {
            self.hireling_calls(game, a);
            self.pet_follows(game, a);
            self.collect_sent(a);
        });
    }

    fn flush_player_tail(&mut self, events: &mut D) {
        events.action().flush_player_tail();
    }

    fn after_tick(&mut self, game: &mut Game, events: &mut D) {
        // Each step's sends join the outbox before the next step runs
        // (production order).
        // The town-leave refreshes of this tick's client pass (the level
        // changes): the left store is gone in this frame.
        let _span = crate::perf::span("world.after_tick");
        self.town_leaves(game, events);
        self.collect_sent(events);
        let sent = self.desk(game, events, quest_objects);
        self.inv_sent.extend(sent);
        self.collect_sent(events);
        let before = self.outbox.len();
        self.run_quest_events(game, events);
        self.collect_sent(events);
        // The game sends an event raised inside a monster's AI at that point:
        // move what the events sent to the mark it left.
        if let Some(at) = self.outbox[..before]
            .iter()
            .position(|(_, b)| b.len() == 5 && b[0] == d2_sim::wiring::action::QUEST_EVENT_MARK)
        {
            let new = self.outbox.split_off(before);
            self.outbox.remove(at);
            let tail = self.outbox.split_off(at);
            self.outbox.extend(new);
            self.outbox.extend(tail);
        }
    }

    /// The quest control on the desk's economy and rest
    /// ([`EconomyQuests`]). The mercenary rewards an Act I
    /// quest grants are queued during the call, with
    /// the sends that follow them
    /// ([`d2_sim::wiring::economy::QuestDeferred`]), and run on the NPC
    /// control block right after it
    /// ([`d2_sim::world::npc::NpcControl::quest_mercenary`] with the desk
    /// as its world), then the queued sends, before the result is
    /// returned: the reward's 0x50 precedes the text refresh's 0x27 / 0x29. The order of
    /// `Desk::quest_message`, here for every quest call. A reward's NPC
    /// error goes to the interaction state's errors, as there.
    ///
    /// The object module's queued quest routes run before the call and
    /// after it ([`quest_objects`]).
    fn quests<C: QuestCall>(&mut self, game: &mut Game, events: &mut D, call: C) -> Option<C::Out> {
        let (out, sent) = self.desk(game, events, |desk, ctl, mut inv| {
            let mut sent = quest_objects(desk, ctl, inv.as_deref_mut());
            let (out, s) = quest_call(desk, ctl, inv.as_deref_mut(), |q, w| call.call(q, w));
            sent.extend(s);
            sent.extend(quest_objects(desk, ctl, inv));
            (out, sent)
        });
        self.inv_sent.extend(sent);
        Some(out)
    }

    /// The cube on this world's economy and inventory model.
    fn cube<C: CubeCall>(&mut self, game: &mut Game, events: &mut D, call: C) -> Option<C::Out> {
        self.cube.as_ref()?;
        let out = self.with_economy(game, events, |econ, p| {
            let parts = p.cube.as_deref_mut().expect("checked above");
            let inv = p.inventory.as_deref_mut();
            let mut quests = CubeQuestHost {
                quests: &mut *p.quests,
                rest: &mut *p.rest,
            };
            call.call(econ, parts, inv, Some(&mut quests))
        });
        // The game runs the staff hook inside the transmute; here it runs
        // right after the call.
        self.run_cube_staff(game, events);
        Some(out)
    }

    /// The item moves on this world's economy and inventory parts (lent
    /// out of the world for the call).
    fn moves<C: MoveCall>(&mut self, game: &mut Game, events: &mut D, call: C) -> Option<C::Out> {
        let mut inv = self.inventory.take()?;
        // The hireling lists lent for the 0x61 give's hireling, owner
        // test and swap (`d2_sim::wiring::inventory::merc`), read only.
        inv.state.hirelings = self
            .state
            .hireling_tables
            .is_some()
            .then(|| self.state.hirelings.clone());
        let mut picks = Vec::new();
        let out = self.with_economy(game, events, |econ, parts| {
            // D2RUST's own, unverified (D1): the preview rest reads the
            // places staged here (`MoveRest::stage`).
            let mut places = Vec::new();
            // The players' quest flags of the game's difficulty, staged
            // for the quest-item uses of 0x20 (`ass`, `xyz`, `tr2`) and
            // written back after the call.
            let difficulty = usize::from(econ.fields.difficulty).min(2);
            let mut flags = Vec::new();
            let mut by_owner = Vec::new();
            for u in econ
                .game
                .lists
                .units_of_type(d2_sim::units::UnitType::Player)
            {
                let Some(guid) = econ.units.get(u).map(|r| r.guid) else {
                    continue;
                };
                let owner = d2_sim::items::moves::Owner::player(guid);
                if let Some(q) = parts.rest.quests(u) {
                    flags.push((owner, q.flags[difficulty]));
                    by_owner.push((owner, u));
                }
            }
            inv.rest.stage_quest_flags(&flags);
            for u in econ
                .game
                .lists
                .units_of_type(d2_sim::units::UnitType::Player)
            {
                let Some(r) = econ.units.get(u) else { continue };
                places.push(super::super::items::moves::StagedPlace {
                    owner: d2_sim::items::moves::Owner::player(r.guid),
                    pos: econ.hooks.path_position(u),
                    room: econ.game.lists.unit(u).and_then(|e| e.room()),
                    used_skill: econ.hooks.used_skill_of(u).is_some(),
                });
            }
            for (&u, it) in &inv.state.items {
                places.push(super::super::items::moves::StagedPlace {
                    owner: d2_sim::items::moves::Owner::item(it.guid),
                    // A drop made by the treasure walk keeps its spot in the
                    // static path, not in the item data (q-a4-quest-items).
                    pos: if (it.x, it.y) == (0, 0) {
                        econ.hooks.path_position(u)
                    } else {
                        (it.x, it.y)
                    },
                    room: econ.game.lists.unit(u).and_then(|e| e.room()),
                    used_skill: false,
                });
            }
            let format = d2_sim::items::ItemGame::item_format(&*econ.fields);
            inv.rest.stage(&places, format);
            // The room boxes a drop next to a player can land in: each
            // player's room, then the rooms next to it.
            let mut rooms: Vec<(d2_sim::units::RoomId, d2_sim::drlg::TileRect)> = Vec::new();
            for p in &places {
                let Some(room) = p.room.filter(|_| p.owner.is_player()) else {
                    continue;
                };
                let adjacent = econ
                    .game
                    .lists
                    .room(room)
                    .map(|r| r.adjacent.clone())
                    .unwrap_or_default();
                for r in std::iter::once(room).chain(adjacent) {
                    if rooms.iter().any(|&(known, _)| known == r) {
                        continue;
                    }
                    if let Some(t) = econ.hooks.drlg.subtiles(econ.game, r) {
                        rooms.push((r, t));
                    }
                }
            }
            inv.rest.stage_rooms(&rooms);
            // The potion's Blood Golem share: each player's first golem pet (pet type 3, the
            // first node without the skip flag whose monster exists,
            // `player::pets::first_pet`) and `Param6` of the skill in
            // monstats row 290's `Skill1`.
            let mut golems = Vec::new();
            for u in econ
                .game
                .lists
                .units_of_type(d2_sim::units::UnitType::Player)
            {
                let Some(guid) = econ.units.get(u).map(|r| r.guid) else {
                    continue;
                };
                let pet = econ
                    .hooks
                    .pet_lists
                    .get(&u)
                    .and_then(|l| l.entries.get(3))
                    .and_then(|e| {
                        e.nodes
                            .iter()
                            .find(|n| n.flags & d2_sim::player::pets::NODE_SKIP == 0)
                    })
                    .and_then(|n| u32::try_from(n.guid).ok())
                    .filter(|&g| {
                        econ.game
                            .lists
                            .find_unit(d2_sim::units::UnitType::Monster, g)
                            .is_some()
                    });
                if let Some(g) = pet {
                    golems.push((
                        d2_sim::items::moves::Owner::player(guid),
                        d2_sim::items::moves::Owner::monster(g),
                    ));
                }
            }
            let t = &econ.hooks.tables;
            let param6 = t
                .combat
                .monstats
                .get(usize::from(d2_sim::wiring::inventory::potion::BLOOD_GOLEM))
                .map(|m| m.skill1 as i16)
                .filter(|&s| s >= 0)
                .and_then(|s| t.skills.skills.get(s as usize))
                .map(|r| r.param6 as i32);
            inv.rest.stage_pets(&golems, param6);
            lend_skills(econ, &mut inv);
            let out = call.call(econ, &mut inv);
            return_skills(econ, &mut inv, true);
            for (owner, quest, flag, on) in inv.rest.take_quest_flag_writes() {
                let Some(&(_, u)) = by_owner.iter().find(|(o, _)| *o == owner) else {
                    continue;
                };
                if let Some(q) = parts.rest.quests(u) {
                    let f = &mut q.flags[difficulty];
                    if on {
                        f.set(quest, flag);
                    } else {
                        f.clear(quest, flag);
                    }
                }
            }
            for (owner, guid) in inv.rest.take_picked_items() {
                let Some(&(_, u)) = by_owner.iter().find(|(o, _)| *o == owner) else {
                    continue;
                };
                if let Some(item) = econ
                    .game
                    .lists
                    .find_unit(d2_sim::units::UnitType::Item, guid)
                {
                    picks.push((u, item));
                }
            }
            out
        });
        self.item_picks.extend(picks);
        inv.state.hirelings = None;
        self.inventory = Some(inv);
        let mut out = out;
        let before = self.outbox.len();
        self.run_item_picks(game, events);
        // The game runs the hook inside the pick-up: its messages lead the
        // call's own.
        let quest: Vec<_> = self.outbox.drain(before..).collect();
        let left = C::front_sent(&mut out, quest);
        self.outbox.extend(left);
        Some(out)
    }

    /// The closing steps of the weapon switch on the player's server
    /// list: a selecting side selects its saved pair (its 0x23 goes at
    /// once) and every side queues its record.
    fn weapon_switched(
        &mut self,
        events: &mut D,
        player: UnitId,
        guid: u32,
        sides: [d2_sim::wiring::inventory::swap::SwitchSide; 2],
    ) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        use d2_sim::wiring::inventory::swap::SwitchSide;
        let hooks = events.action().hooks();
        let tables = hooks.tables.clone();
        let Some(list) = hooks.skill_lists.get_mut(&player) else {
            return (Vec::new(), Vec::new());
        };
        let msg = |left: bool, (skill, owner): (i32, i32)| {
            let m = d2_sim::units::messages::set_skill(
                0,
                guid,
                u8::from(left),
                skill as u16,
                owner as u32,
            );
            m.to_vec()
        };
        let (mut direct, mut records) = (Vec::new(), Vec::new());
        for (left, side) in [(true, sides[0]), (false, sides[1])] {
            if let SwitchSide::Select(s) = side {
                if list.select(&tables.skills.skills, left, s.0, s.1).is_ok() {
                    direct.push(msg(left, s));
                }
            }
            records.push(msg(left, side.record()));
        }
        (direct, records)
    }

    /// The skill handlers, then the pet follows their placements queued
    /// ([`WiredWorld::pet_follows`]).
    fn skill(&mut self, call: SkillCall<'_, D>) -> Option<SkillHandled> {
        let SkillCall {
            game,
            events,
            client,
            msg,
            staged,
        } = call;
        let call = SkillCall {
            game: &mut *game,
            events: &mut *events,
            client,
            msg,
            staged,
        };
        let out = WorldHost::<D>::skill(&mut self.action, call);
        self.pet_deaths(game, events);
        self.hireling_calls(game, events);
        self.pet_follows(game, events);
        out
    }

    /// The action wiring's provider with this host's answer, for 0x46 /
    /// 0x47, of the hireling list
    /// (`NpcWorld::pet` on the desk);
    /// then the pet follows a warp queued ([`WiredWorld::pet_follows`]).
    fn player(
        &mut self,
        game: &mut Game,
        events: &mut D,
        run: PlayerRun<'_>,
    ) -> Option<PlayerOutcome> {
        let p = run.player;
        let hireling = matches!(run.msg.first(), Some(0x46 | 0x47)).then(|| {
            self.desk(game, events, |desk, _, _| {
                NpcWorld::pet(desk, p, d2_sim::world::hirelings::PET_HIRELING, 0)
            })
        });
        let facts = HostFacts { hireling };
        let out = player::action::run(game, events, &run, facts);
        self.hireling_calls(game, events);
        self.pet_follows(game, events);
        Some(out)
    }

    fn unit_positions(&mut self, events: &mut D, units: &[UnitId]) -> Vec<(UnitId, (i32, i32))> {
        WorldHost::<D>::unit_positions(&mut self.action, events, units)
    }

    fn walk(&mut self, game: &mut Game, events: &mut D, call: WalkCall) -> Option<WalkResult> {
        self.drop_queued(call.player);
        self.item_queued.retain(|q| q.0 != call.player);
        self.object_queued.retain(|q| q.0 != call.player);
        // The position resync 0x5F is no new walk request: the pending
        // tile 0x13 survives it.
        if call.id != crate::adapters::handlers::walk::RESYNC_ID {
            self.tile_queued.retain(|q| q.0 != call.player);
        }
        let out = self.lend_quests(events, |a, ev| WorldHost::<D>::walk(a, game, ev, call));
        self.pet_deaths(game, events);
        self.hireling_calls(game, events);
        self.pet_follows(game, events);
        out
    }

    fn player_gate(
        &mut self,
        game: &Game,
        events: &mut D,
        unit: UnitId,
    ) -> Option<crate::seams::PlayerGate> {
        WorldHost::<D>::player_gate(&mut self.action, game, events, unit)
    }

    fn live_facts(
        &mut self,
        game: &Game,
        events: &mut D,
        unit: UnitId,
    ) -> Option<crate::adapters::UnitFacts> {
        if let Some(f) = WorldHost::<D>::live_facts(&mut self.action, game, events, unit) {
            return Some(f);
        }
        self.item_facts(game, events, unit)
    }

    fn vitals_sync(
        &mut self,
        game: &mut Game,
        events: &mut D,
        client: d2_sim::units::ClientId,
        staged: (u16, u16),
        queued: bool,
    ) -> Option<Vec<Vec<u8>>> {
        WorldHost::<D>::vitals_sync(&mut self.action, game, events, client, staged, queued)
    }

    /// The outbox in production order ([`WiredWorld::collect_sent`]):
    /// the tick's steps in turn; for a handled message, the action
    /// wiring's sends (waypoints, tick paths), then what the inventory
    /// rules queued in vendor calls, then the rest's (NPC, vendor and
    /// quest messages); one system runs per message, so the systems never
    /// interleave. A vendor call's inventory messages (a targeting
    /// reset's 0x3F, placement and 0x9D sends) come before its 0x2A, the
    /// last call of each buy pass, sell or repair.
    fn take_sent(&mut self, events: &mut D) -> Vec<(UnitId, Vec<u8>)> {
        self.collect_sent(events);
        std::mem::take(&mut self.outbox)
    }

    fn take_faults(&mut self) -> Vec<WorldError> {
        std::mem::take(&mut self.late_faults)
    }

    fn take_client_pass_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.shown_sent)
    }

    /// The action wiring's object host tick, and [`WiredWorld::now`] (the
    /// vendors' clock): both read the host's millisecond clock
    /// (`GetTickCount`), once per host frame.
    ///
    /// This host holds the quest control, so the object module's quest
    /// routes are queued for it from here on
    /// (`ActionSim::route_quest_objects`; the game's creator turns it on
    /// at creation, before the first object).
    fn host_tick(&mut self, events: &mut D, ms: u32) {
        events.action().route_quest_objects();
        let h = &mut events.action().sys.hooks;
        h.pet_follows.get_or_insert_with(Vec::new);
        h.defer_player_tail = true;
        h.pet_deaths.get_or_insert_with(Vec::new);
        h.owner_deaths.get_or_insert_with(Vec::new);
        h.hireling_calls.get_or_insert_with(Vec::new);
        WorldHost::<D>::host_tick(&mut self.action, events, ms);
        self.now = ms;
    }

    /// The host calls the quest rules raised since the last take
    /// (game end, save pass), in call order,
    /// from the quest control (a lent control's come back with it,
    /// [`WiredWorld::lend_quests`]).
    fn take_host_requests(&mut self) -> Vec<HostRequest> {
        self.quests.take_host_requests()
    }

    fn fault(&mut self, fault: WorldFault) {
        self.action.faults.push(fault);
    }
}

/// The players' skill lists lent to the inventory rest for a call and the
/// item-granted entries synced after it ([`return_skills`];
/// `sync` false: the caller synced already).
pub(super) fn lend_skills<X: Pending>(econ: &mut Economy<'_, ActionHooks<X>>, inv: &mut InvParts) {
    let mut staged = std::collections::BTreeMap::new();
    // Changes made outside a lend are not called back here.
    econ.hooks.oskill_changes.clear();
    for u in econ
        .game
        .lists
        .units_of_type(d2_sim::units::UnitType::Player)
    {
        let Some(r) = econ.units.get(u) else { continue };
        let (guid, class) = (r.guid, r.class as i32);
        if let Some(list) = econ.hooks.skill_lists.remove(&u) {
            staged.insert(
                d2_sim::items::moves::Owner::player(guid),
                super::super::items::moves::preview_skills::StagedList {
                    list,
                    unit: u,
                    class,
                },
            );
        }
    }
    inv.rest.stage_skills(
        super::super::items::moves::preview_skills::SkillStage {
            tables: econ.hooks.tables.clone(),
            lists: staged,
        },
        &inv.tables,
    );
}

/// Takes the lent skill lists back: the stat 97 / 107 callback
/// for the items worn, its 0x21 queued with the rest's
/// messages, the lists returned to the action hooks.
pub(super) fn return_skills<X: Pending>(
    econ: &mut Economy<'_, ActionHooks<X>>,
    inv: &mut InvParts,
    sync: bool,
) {
    if let Some(mut st) = inv.rest.take_skills() {
        let mut sent = Vec::new();
        let changes = std::mem::take(&mut econ.hooks.oskill_changes);
        let owners: Vec<_> = st.lists.keys().copied().collect();
        for o in owners.into_iter().filter(|_| sync) {
            let unit = st.lists[&o].unit;
            let mine: Vec<_> = changes
                .iter()
                .filter(|c| c.0 == unit)
                .map(|c| (c.1, c.2, c.3))
                .collect();
            super::super::items::moves::preview_skills::sync_oskills(
                &mut st,
                o,
                &mine,
                |sl, stat, skill| econ.stats.unit_total(sl.unit, stat, skill as u16),
                |sl| econ.stats.has_state(sl.unit, SKILL_SHRINE_STATE),
                &mut sent,
            );
        }
        inv.rest.queue_sent(sent);
        for (_, sl) in st.lists {
            econ.hooks.skill_lists.insert(sl.unit, sl.list);
        }
    }
}
