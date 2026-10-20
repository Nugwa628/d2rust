//! Conservation across observed item-event windows. Pure transfers (pickup,
//! drop, stash, equip and trade) conserve the global quantity and gold totals.
//! Sources/sinks (vendors, cube, use, drops, expiry and explicit dev setup)
//! open only the corresponding side of the ledger for that tick.
//!
//! Repair refills quantity items: a repaired throwable stack is set back to
//! its maximum stack, so a repair request is a declared quantity source for
//! the items that already exist (it never brings in new items).
use super::{v, Violation};
use crate::app::single_player::Sim;
use crate::scenario_script::{Scenario, StepMsg};
use d2_sim::units::UnitType;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// All live items, including NPC, ground, socket and player inventories.
    /// GUID -> (item table row, quantity). Gold piles are in `gold` instead.
    pub items: BTreeMap<u32, (usize, i64)>,
    /// Item GUID -> the room it is in. Player-carried items have no room.
    pub rooms: BTreeMap<u32, u32>,
    pub gold: i64,
    pub gold_units: BTreeSet<(u8, u32)>,
    /// Gold pile GUID -> its amount in gold.
    pub gold_piles: BTreeMap<u32, i64>,
    /// The rooms active at the snapshot: a room entering this set brings in
    /// the units that were placed in it (level population).
    pub active_rooms: BTreeSet<u32>,
    /// The active rooms whose units are populated and live. A room entering
    /// this set had its objects and monsters populated, or its remembered
    /// ground units restored, on that tick (`tick::room_body`).
    pub live_rooms: BTreeSet<u32>,
    /// Monster GUID -> the room it is in, to spot a room's population.
    pub monster_rooms: BTreeMap<u32, u32>,
    /// The rooms the players are in: a new ground unit elsewhere is level
    /// population, not a game action.
    pub player_rooms: BTreeSet<u32>,
    pub expiring_gold: i64,
    pub expiring: BTreeSet<u32>,
    pub dead: BTreeSet<(u8, u32)>,
    pub replenishing: BTreeSet<u32>,
}

pub fn snapshot(sim: &Sim) -> Snapshot {
    let mut out = Snapshot::default();
    let sys = &sim.events.action.sys;
    let gold = sim.world.tables.find_code(*b"gld ");
    for act in 0..5u8 {
        for room in sim.game.lists.active_rooms(act) {
            out.active_rooms.insert(room.0);
            if sim
                .game
                .lists
                .room(room)
                .is_some_and(|r| r.populated && r.units_active)
            {
                out.live_rooms.insert(room.0);
            }
        }
    }
    for ty in [UnitType::Player, UnitType::Monster] {
        for id in sim.game.lists.units_of_type(ty) {
            if let Some(unit) = sys.units.get(id) {
                if unit.is_dead() {
                    out.dead.insert((ty as u8, unit.guid));
                }
                if ty == UnitType::Monster {
                    if let Some(room) = sim.game.lists.unit(id).and_then(|e| e.room()) {
                        out.monster_rooms.insert(unit.guid, room.0);
                    }
                }
            }
            if ty == UnitType::Player {
                if let Some(unit) = sys.units.get(id) {
                    out.gold_units.insert((ty as u8, unit.guid));
                }
                if let Some(room) = sim.game.lists.unit(id).and_then(|e| e.room()) {
                    out.player_rooms.insert(room.0);
                }
                out.gold += i64::from(sys.stats.unit_total(id, 14, 0))
                    + i64::from(sys.stats.unit_total(id, 15, 0));
            }
        }
    }
    for id in sim.game.lists.units_of_type(UnitType::Item) {
        let Some(unit) = sys.units.get(id) else {
            continue;
        };
        let Some(item) = sys.hooks.items.get(id) else {
            continue;
        };
        if let Some(room) = sim.game.lists.unit(id).and_then(|e| e.room()) {
            out.rooms.insert(unit.guid, room.0);
        }
        if Some(item.record) == gold {
            let amount = i64::from(sys.stats.unit_total(id, 14, 0));
            out.gold += amount;
            out.gold_piles.insert(unit.guid, amount);
            out.gold_units.insert((UnitType::Item as u8, unit.guid));
            if item.ground_expiry > 0 && item.ground_expiry <= sim.game.frame + 1 {
                out.expiring_gold += amount;
            }
        } else {
            let stackable = sim
                .world
                .tables
                .items
                .get(item.record)
                .is_some_and(|row| row.stackable != 0);
            let quantity = if stackable {
                i64::from(sys.stats.unit_total(id, 70, 0))
            } else {
                1
            };
            out.items.insert(unit.guid, (item.record, quantity));
        }
        if item.ground_expiry > 0 && item.ground_expiry <= sim.game.frame + 1 {
            out.expiring.insert(unit.guid);
        }
        if sys.stats.unit_total(id, 253, 0) > 0 {
            out.replenishing.insert(unit.guid);
        }
    }
    out
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Events {
    pub creates: bool,
    pub destroys: bool,
    pub gold_source: bool,
    pub gold_sink: bool,
    /// Existing stacks may grow (repair refills quantity items); new items
    /// still need `creates`.
    pub refills: bool,
    /// A skill request was sent this tick (see `SKILL_GRACE`).
    pub skill: bool,
}

/// C->S ids and explicit setup steps identify events, not arbitrary total
/// changes. A pickup/drop/trade/stash request grants no exemption.
pub fn events(scenario: &Scenario, tick: u32, sent: &Value, before: &Value) -> Events {
    let mut events = Events::default();
    for (id, count) in sent.as_object().into_iter().flatten() {
        if count.as_u64().unwrap_or(0) <= before[id].as_u64().unwrap_or(0) {
            continue;
        }
        if let Ok(id) = u8::from_str_radix(id, 16) {
            event_id(&mut events, id);
        }
    }
    for step in &scenario.steps {
        // A setup step (poke/dev/spawn) takes effect on its own tick; a sent
        // message lands on the next one.
        if step.tick != tick && step.tick.checked_add(1) != Some(tick) {
            continue;
        }
        match &step.msg {
            StepMsg::Dev(text) => {
                let command = text.split_whitespace().next().unwrap_or("");
                match command {
                    "gold" => {
                        events.gold_source = true;
                        events.gold_sink = true;
                    }
                    "item" | "spawn" => events.creates = true,
                    "warp" => {
                        events.creates = true;
                        events.destroys = true;
                    }
                    _ => {}
                }
            }
            StepMsg::Poke(d2_sim::poke::Directive::Stat { stat: 14 | 15, .. }) => {
                events.gold_source = true;
                events.gold_sink = true;
            }
            StepMsg::Poke(op) => {
                let text = op.to_string();
                match text.split_whitespace().next().unwrap_or("") {
                    "item" => events.creates = true,
                    "warp" => {
                        events.creates = true;
                        events.destroys = true;
                    }
                    _ => {}
                }
            }
            StepMsg::Spawn(_) => events.creates = true,
            StepMsg::Typed { id: 0x4F, fields } => {
                if fields.iter().any(|(name, value)| {
                    name == "button" && matches!(value, crate::scenario_script::Arg::Num(0x18))
                }) {
                    events.creates = true;
                    events.destroys = true;
                }
            }
            StepMsg::Typed { id, .. } => event_id(&mut events, *id),
            StepMsg::Hex(bytes) => {
                if let Some(&id) = bytes.first() {
                    event_id(&mut events, id);
                    if id == 0x4F && bytes.get(1..3) == Some(&[0x18, 0][..]) {
                        events.creates = true;
                        events.destroys = true;
                    }
                }
            }
            _ => {}
        }
    }
    events
}

/// Whether a `goto` poke is still walking at `tick`, or ended on the tick
/// before: from its step's tick through the tick after its result
/// (`finished`: the ticks the scenario's `goto` results were logged on,
/// in order). A `goto` warps and hops the player and populates the rooms
/// it streams, so, like `warp`, its whole walk opens the ledger.
pub fn goto_in_flight(scenario: &Scenario, finished: &[u32], tick: u32) -> bool {
    let started = scenario
        .steps
        .iter()
        .filter(|s| s.tick <= tick)
        .filter(|s| matches!(s.msg, StepMsg::Poke(d2_sim::poke::Directive::Goto(_))))
        .count();
    let done = finished.iter().filter(|&&t| t + 1 < tick).count();
    started > done
}

impl Events {
    /// A player move that loads and unloads rooms: every side open.
    pub fn open_all(&mut self) {
        self.creates = true;
        self.destroys = true;
        self.gold_source = true;
        self.gold_sink = true;
    }
}

fn event_id(events: &mut Events, id: u8) {
    match id {
        // Opening an object can generate treasure; opening a vendor can
        // initialize stock. Neither permits unexplained item losses.
        0x13 => {
            events.creates = true;
            events.gold_source = true;
        }
        0x2F => events.creates = true,
        // Ending the interaction drops the gamble list (temporary vendor
        // items); the store itself is kept.
        0x30 => events.destroys = true,
        // A menu action can open a vendor: trade and gamble initialize the
        // store or gamble list (the other actions cannot create items). The
        // client's send is counted by id only here, so the source is
        // declared for the whole menu action.
        0x38 => events.creates = true,
        // Buying (including a store's replenished copy), selling and repair
        // are declared economic sources/sinks. Pure transfers aren't here.
        0x32 => {
            events.creates = true;
            events.gold_sink = true;
        }
        0x33 => {
            events.creates = true;
            events.destroys = true;
            events.gold_source = true;
        }
        // Identify and hire charge a fee; a hire also creates the mercenary
        // and its equipment.
        0x34 => events.gold_sink = true,
        0x36 => {
            events.creates = true;
            events.gold_sink = true;
        }
        // Repair charges its fee and refills quantity items (throwable
        // stacks go back to their maximum stack).
        0x35 => {
            events.gold_sink = true;
            events.refills = true;
        }
        // Item use consumes quantities; scrolls transferred into a book
        // change table row. Stack splits/merges and sockets conserve totals.
        0x20 | 0x26 | 0x27 => events.destroys = true,
        0x29 => {
            events.creates = true;
            events.destroys = true;
        }
        // Skills may consume ammunition/throwing stacks.
        0x05..=0x11 => {
            events.destroys = true;
            events.skill = true;
        }
        // A quest message runs the NPC's quest script inside the server's
        // handling of that message (`QuestControl::quest_message` dispatches
        // the scroll-message event synchronously), so its effect lands on the
        // tick after the send, which the one-tick message lag already covers.
        // The scripts give reward items (`reward_item`) and take quest items
        // (`delete_item`); they never touch gold (their only stat changes are
        // stat points, skills, life and experience). Creates and destroys are
        // therefore the only exemptions, and only for those two ticks.
        0x31 => {
            events.creates = true;
            events.destroys = true;
        }
        _ => {}
    }
}

/// Ticks after a skill request in which items may appear or vanish: ammunition
/// is spent at the release frame and summons get their gear after the cast,
/// both well after the request tick.
const SKILL_GRACE: u32 = 180;

/// How many observations a room that just loaded keeps the ground-unit
/// grace: an object's init can create its contents a few ticks later, and a
/// level's remembered ground units stream in after the player does.
const LOAD_GRACE: u32 = 15;

#[derive(Default)]
pub struct Conservation {
    previous: Option<Snapshot>,
    skill_ticks: u32,
    /// The previous tick's events: a sent message is handled by the server
    /// on the next tick, so its source/sink also covers that tick.
    last_events: Events,
    /// The rooms still inside the load grace, with the observations left.
    recent_rooms: BTreeMap<u32, u32>,
}

impl Conservation {
    pub fn observe(&mut self, now: Snapshot, mut events: Events) -> Vec<Violation> {
        // A C→S message lands on the next server tick: the source/sink the
        // previous tick's message opened covers this one too.
        let incoming = events;
        events.creates |= self.last_events.creates;
        events.destroys |= self.last_events.destroys;
        events.gold_source |= self.last_events.gold_source;
        events.gold_sink |= self.last_events.gold_sink;
        events.refills |= self.last_events.refills;
        self.last_events = incoming;
        if events.skill {
            self.skill_ticks = SKILL_GRACE;
        }
        if self.skill_ticks > 0 {
            self.skill_ticks -= 1;
            events.creates = true;
            events.destroys = true;
        }
        let Some(previous) = self.previous.replace(now.clone()) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        // Death creates treasure and can charge the player's death penalty.
        if !now.dead.is_subset(&previous.dead) {
            events.creates = true;
            events.gold_source = true;
            events.gold_sink = true;
        }
        // A room that became active, or whose population just arrived (new
        // monsters in it), brings in the units placed with that level
        // generation. Those were not created by a game action, so their
        // quantities open the source side of the ledger.
        let mut populated: BTreeSet<u32> = BTreeSet::new();
        for (&guid, &room) in &now.monster_rooms {
            if !previous.monster_rooms.contains_key(&guid) {
                populated.insert(room);
            }
        }
        // The rooms that finished loading now. A room the player just
        // entered is loading too: its population can arrive after the
        // player does.
        let loading: BTreeSet<u32> = populated
            .iter()
            .copied()
            .chain(now.active_rooms.difference(&previous.active_rooms).copied())
            .chain(now.live_rooms.difference(&previous.live_rooms).copied())
            .chain(now.player_rooms.difference(&previous.player_rooms).copied())
            .collect();
        // A room keeps the grace for LOAD_GRACE observations after it loads:
        // its ground units may stream in after the player is already there.
        let mut recent = std::mem::take(&mut self.recent_rooms);
        for &room in &loading {
            recent.insert(room, LOAD_GRACE);
        }
        let in_grace = |room: u32| recent.get(&room).is_some_and(|left| *left > 0);
        let population_room = |room: u32| {
            let player_there =
                now.player_rooms.contains(&room) || previous.player_rooms.contains(&room);
            !player_there
                || populated.contains(&room)
                || (now.active_rooms.contains(&room) && !previous.active_rooms.contains(&room))
                || (now.live_rooms.contains(&room) && !previous.live_rooms.contains(&room))
                || in_grace(room)
        };
        let mut population: BTreeMap<usize, i64> = BTreeMap::new();
        for (&guid, &(row, quantity)) in &now.items {
            if previous.items.contains_key(&guid) {
                continue;
            }
            let Some(&room) = now.rooms.get(&guid) else {
                // A ground unit whose room link has not landed yet, or an
                // item created while the level loads: while a level is
                // loading, that is its population.
                if !loading.is_empty() || !recent.is_empty() {
                    *population.entry(row).or_default() += quantity;
                }
                continue;
            };
            if population_room(room) {
                *population.entry(row).or_default() += quantity;
            }
        }
        let mut population_gold = 0i64;
        for (&guid, &amount) in &now.gold_piles {
            if previous.gold_piles.contains_key(&guid) {
                continue;
            }
            let Some(&room) = now.rooms.get(&guid) else {
                if !loading.is_empty() || !recent.is_empty() {
                    population_gold += amount;
                }
                continue;
            };
            if population_room(room) {
                population_gold += amount;
            }
        }
        // The ground units of a room that left the active set are freed with
        // its level: leaving a level unloads what lay on it, which is not a
        // game action on those items.
        let unloaded = |guid: &u32| {
            previous.rooms.get(guid).is_some_and(|room| {
                previous.active_rooms.contains(room) && !now.active_rooms.contains(room)
            })
        };
        let mut unloaded_items: BTreeMap<usize, i64> = BTreeMap::new();
        for (guid, &(row, quantity)) in &previous.items {
            if !now.items.contains_key(guid) && unloaded(guid) {
                *unloaded_items.entry(row).or_default() += quantity;
            }
        }
        let unloaded_gold: i64 = previous
            .gold_piles
            .iter()
            .filter(|(guid, _)| !now.gold_piles.contains_key(guid) && unloaded(guid))
            .map(|(_, &amount)| amount)
            .sum();
        // A refill grows stacks that already existed; only that growth is
        // covered, never a new item.
        let mut refilled: BTreeMap<usize, i64> = BTreeMap::new();
        if events.refills {
            for (guid, &(row, quantity)) in &now.items {
                if let Some(&(old_row, old_quantity)) = previous.items.get(guid) {
                    if old_row == row && quantity > old_quantity {
                        *refilled.entry(row).or_default() += quantity - old_quantity;
                    }
                }
            }
        }
        let totals = |s: &Snapshot| {
            let mut result = BTreeMap::<usize, i64>::new();
            for &(row, quantity) in s.items.values() {
                *result.entry(row).or_default() += quantity;
            }
            result
        };
        let old = totals(&previous);
        let new = totals(&now);
        for row in old
            .keys()
            .chain(new.keys())
            .copied()
            .collect::<BTreeSet<_>>()
        {
            let before = old.get(&row).copied().unwrap_or(0);
            let after = new.get(&row).copied().unwrap_or(0);
            let expiry = previous
                .expiring
                .iter()
                .filter_map(|g| previous.items.get(g))
                .filter(|(r, _)| *r == row)
                .map(|(_, q)| q)
                .sum::<i64>();
            let replenish = now
                .replenishing
                .iter()
                .any(|g| now.items.get(g).is_some_and(|(r, _)| *r == row));
            let source = before
                + population.get(&row).copied().unwrap_or(0)
                + refilled.get(&row).copied().unwrap_or(0);
            if (after > source && !events.creates && !replenish)
                || (after < before - expiry - unloaded_items.get(&row).copied().unwrap_or(0)
                    && !events.destroys)
            {
                let units = previous
                    .items
                    .iter()
                    .chain(now.items.iter())
                    .filter(|(_, (r, _))| *r == row)
                    .map(|(g, _)| *g)
                    .collect::<BTreeSet<_>>();
                out.push(v("conservation",format!("conservation:items:{row}:{units:?}"),format!("item row {row}: quantity {before} -> {after}, without a matching source/sink event")));
            }
        }
        if (now.gold > previous.gold + population_gold && !events.gold_source)
            || (now.gold < previous.gold - previous.expiring_gold - unloaded_gold
                && !events.gold_sink)
        {
            out.push(v(
                "conservation",
                "conservation:gold".into(),
                format!(
                    "global gold {} -> {} without a matching source/sink event",
                    previous.gold, now.gold
                ),
            ));
            out.last_mut().unwrap().units = previous
                .gold_units
                .union(&now.gold_units)
                .copied()
                .collect();
        }
        self.recent_rooms = recent
            .into_iter()
            .filter_map(|(room, left)| (left > 1).then_some((room, left - 1)))
            .collect();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(guid: u32, quantity: i64, gold: i64) -> Snapshot {
        Snapshot {
            items: [(guid, (1, quantity))].into(),
            gold,
            ..Default::default()
        }
    }
    /// Observes an unchanged state until the load grace has run out.
    fn pass_grace(ledger: &mut Conservation, state: Snapshot) {
        for _ in 0..LOAD_GRACE {
            assert!(ledger.observe(state.clone(), Events::default()).is_empty());
        }
    }
    #[test]
    fn a_pure_transfer_conserves_items_and_gold() {
        let mut ledger = Conservation::default();
        assert!(ledger
            .observe(state(7, 10, 1234), Events::default())
            .is_empty());
        // A stack split changes GUIDs, never total quantity.
        let mut split = state(7, 6, 1234);
        split.items.insert(8, (1, 4));
        assert!(ledger.observe(split, Events::default()).is_empty());
        let failures = ledger.observe(state(7, 5, 1200), Events::default());
        assert_eq!(failures.len(), 2);
        assert!(failures.iter().all(|v| v.kind == "conservation"));
    }
    #[test]
    fn creation_and_destruction_need_a_corresponding_event() {
        let mut ledger = Conservation::default();
        ledger.observe(state(7, 10, 100), Events::default());
        assert!(ledger
            .observe(
                state(7, 11, 90),
                Events {
                    creates: true,
                    gold_sink: true,
                    ..Default::default()
                }
            )
            .is_empty());
        assert!(ledger
            .observe(
                state(7, 5, 100),
                Events {
                    destroys: true,
                    gold_source: true,
                    ..Default::default()
                }
            )
            .is_empty());
        assert_eq!(ledger.observe(state(7, 6, 100), Events::default()).len(), 1);
    }
    #[test]
    fn a_skill_request_covers_its_later_ammo_and_summon_gear() {
        let mut ledger = Conservation::default();
        ledger.observe(state(7, 10, 0), Events::default());
        let mut cast = Events::default();
        event_id(&mut cast, 0x0C);
        assert!(ledger.observe(state(7, 10, 0), cast).is_empty());
        assert!(ledger.observe(state(7, 9, 0), Events::default()).is_empty());
        let mut gear = state(7, 9, 0);
        gear.items.insert(8, (2, 1));
        assert!(ledger.observe(gear.clone(), Events::default()).is_empty());
        for _ in 0..SKILL_GRACE {
            ledger.observe(gear.clone(), Events::default());
        }
        assert_eq!(ledger.observe(state(7, 9, 0), Events::default()).len(), 1);
    }
    #[test]
    fn a_vendor_menu_action_declares_the_stock_it_initializes() {
        let mut event = Events::default();
        event_id(&mut event, 0x38);
        assert!(event.creates);
        assert!(!event.destroys && !event.gold_source && !event.gold_sink);
        // Trade and gamble open stock; the ledger accepts the new rows.
        let mut ledger = Conservation::default();
        ledger.observe(state(7, 10, 100), Events::default());
        let mut stock = state(7, 10, 100);
        stock.items.insert(8, (2, 3));
        assert!(ledger.observe(stock, event).is_empty());
    }

    #[test]
    fn a_goto_opens_the_ledger_until_the_tick_after_its_result() {
        let scenario = Scenario::parse(
            "scenario 2
tick_rate 60
name g
seed 7
end 40
char class 0
             at 1 poke goto preset 37 2:16
at 39 checkpoint
",
        )
        .unwrap();
        // Not yet started, then walking (result not logged yet).
        assert!(!goto_in_flight(&scenario, &[], 0));
        assert!(goto_in_flight(&scenario, &[], 1));
        assert!(goto_in_flight(&scenario, &[], 20));
        // Logged on tick 5: open on 5 and 6, closed from 7.
        assert!(goto_in_flight(&scenario, &[5], 6));
        assert!(!goto_in_flight(&scenario, &[5], 7));
        // The goto's populated gold passes with the opened ledger only.
        let mut ledger = Conservation::default();
        ledger.observe(state(7, 10, 124), Events::default());
        let mut open = Events::default();
        open.open_all();
        assert!(ledger.observe(state(7, 10, 878), open).is_empty());
        // The opened tick also covers the next one (a message's landing).
        assert!(ledger
            .observe(state(7, 10, 878), Events::default())
            .is_empty());
        assert_eq!(
            ledger.observe(state(7, 10, 900), Events::default()).len(),
            1
        );
    }

    #[test]
    fn transfer_requests_never_open_the_ledger() {
        for id in [
            0x16, 0x17, 0x18, 0x19, 0x1A, 0x1C, 0x21, 0x22, 0x23, 0x24, 0x28, 0x2A, 0x4F, 0x50,
            0x61, 0x63,
        ] {
            let mut event = Events::default();
            event_id(&mut event, id);
            assert!(!event.creates && !event.destroys && !event.gold_source && !event.gold_sink);
        }
    }
    #[test]
    fn a_newly_active_rooms_population_is_a_source() {
        let mut ledger = Conservation::default();
        let mut before = state(7, 10, 0);
        before.active_rooms.insert(1);
        before.player_rooms.insert(1);
        ledger.observe(before, Events::default());
        // A gold pile and an item appear in room 2, which just became active.
        let mut after = state(7, 10, 84);
        after.active_rooms = [1, 2].into();
        after.player_rooms.insert(1);
        after.gold_piles.insert(9, 84);
        after.gold_units.insert((UnitType::Item as u8, 9));
        after.rooms.insert(9, 2);
        after.items.insert(10, (3, 2));
        after.rooms.insert(10, 2);
        assert!(ledger.observe(after, Events::default()).is_empty());

        // The same new pile in an already active room still needs a source.
        let mut ledger = Conservation::default();
        let mut before = state(7, 10, 0);
        before.active_rooms.insert(2);
        before.player_rooms.insert(2);
        ledger.observe(before, Events::default());
        let mut after = state(7, 10, 84);
        after.active_rooms.insert(2);
        after.player_rooms.insert(2);
        after.gold_piles.insert(9, 84);
        after.gold_units.insert((UnitType::Item as u8, 9));
        after.rooms.insert(9, 2);
        let failures = ledger.observe(after, Events::default());
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].sig, "conservation:gold");
    }
    #[test]
    fn a_quest_message_covers_only_its_scripts_items_for_two_ticks() {
        let mut quest = Events::default();
        event_id(&mut quest, 0x31);
        assert!(quest.creates && quest.destroys);
        assert!(!quest.gold_source && !quest.gold_sink);

        // A reward item and a taken quest item right after the message.
        let mut ledger = Conservation::default();
        ledger.observe(state(7, 10, 100), Events::default());
        ledger.observe(state(7, 10, 100), quest);
        let mut after = state(7, 10, 100);
        after.items.insert(8, (2, 1));
        assert!(ledger.observe(after.clone(), Events::default()).is_empty());

        // The exemption ends with the message's two ticks: a later create or
        // destroy is a dupe or a loss again.
        let mut later = after.clone();
        later.items.insert(9, (3, 1));
        let failures = ledger.observe(later, Events::default());
        assert_eq!(failures.len(), 1);
        assert!(failures[0].sig.starts_with("conservation:items:3"));
        let mut ledger = Conservation::default();
        ledger.observe(state(7, 10, 100), Events::default());
        ledger.observe(state(7, 10, 100), quest);
        ledger.observe(state(7, 10, 100), Events::default());
        ledger.observe(state(7, 10, 100), Events::default());
        let mut loss = state(7, 10, 100);
        loss.items.clear();
        assert_eq!(ledger.observe(loss, Events::default()).len(), 1);
    }
    #[test]
    fn a_quest_message_never_explains_a_gold_change() {
        let mut quest = Events::default();
        event_id(&mut quest, 0x31);
        for gold in [150, 50] {
            let mut ledger = Conservation::default();
            ledger.observe(state(7, 10, 100), Events::default());
            ledger.observe(state(7, 10, 100), quest);
            let failures = ledger.observe(state(7, 10, gold), Events::default());
            assert_eq!(failures.len(), 1, "gold 100 -> {gold}");
            assert_eq!(failures[0].sig, "conservation:gold");
        }
    }
    #[test]
    fn service_requests_charge_their_fees() {
        for (id, creates) in [(0x34, false), (0x35, false), (0x36, true)] {
            let mut event = Events::default();
            event_id(&mut event, id);
            assert!(event.gold_sink, "0x{id:02X} charges a fee");
            assert!(!event.gold_source);
            assert_eq!(event.creates, creates, "0x{id:02X}");
        }
    }
    #[test]
    fn repair_refills_quantity_items_only() {
        let mut repair = Events::default();
        event_id(&mut repair, 0x35);
        assert!(repair.refills && !repair.creates);
        // A repaired javelin stack 3 -> 20 (fee paid) on the tick the
        // request lands is a declared source.
        let mut ledger = Conservation::default();
        ledger.observe(state(7, 3, 100), Events::default());
        ledger.observe(state(7, 3, 100), repair);
        assert!(ledger
            .observe(state(7, 20, 90), Events::default())
            .is_empty());
        // Without the request the same growth is unexplained.
        let mut ledger = Conservation::default();
        ledger.observe(state(7, 3, 100), Events::default());
        let failures = ledger.observe(state(7, 20, 100), Events::default());
        assert_eq!(failures.len(), 1);
        // A repair never brings in a new item of that row.
        let mut ledger = Conservation::default();
        ledger.observe(state(7, 3, 100), repair);
        let mut after = state(7, 3, 90);
        after.items.insert(8, (1, 5));
        let failures = ledger.observe(after, Events::default());
        assert_eq!(failures.len(), 1);
        assert!(failures[0].sig.starts_with("conservation:items:1"));
    }
    #[test]
    fn a_message_covers_the_tick_it_lands_on() {
        let mut ledger = Conservation::default();
        ledger.observe(state(7, 10, 100), Events::default());
        ledger.observe(
            state(7, 10, 100),
            Events {
                creates: true,
                gold_sink: true,
                ..Default::default()
            },
        );
        let after = state(7, 11, 90);
        assert!(ledger.observe(after.clone(), Events::default()).is_empty());
        let mut later = after;
        later.items.insert(8, (2, 1));
        let failures = ledger.observe(later, Events::default());
        assert_eq!(failures.len(), 1);
    }
    #[test]
    fn ending_the_chat_drops_the_gamble_list() {
        let mut event = Events::default();
        event_id(&mut event, 0x30);
        assert!(event.destroys);
        assert!(!event.creates && !event.gold_source && !event.gold_sink);
    }
    #[test]
    fn a_rooms_population_with_its_monsters_is_a_source() {
        let mut ledger = Conservation::default();
        let mut before = state(7, 10, 0);
        before.active_rooms.insert(2);
        before.player_rooms.insert(2);
        ledger.observe(before, Events::default());
        let mut after = state(7, 10, 84);
        after.active_rooms.insert(2);
        after.player_rooms.insert(2);
        after.monster_rooms.insert(20, 2);
        after.gold_piles.insert(9, 84);
        after.gold_units.insert((UnitType::Item as u8, 9));
        after.rooms.insert(9, 2);
        assert!(ledger.observe(after, Events::default()).is_empty());
    }
    #[test]
    fn the_ground_units_of_an_unloaded_level_are_not_a_loss() {
        let mut ledger = Conservation::default();
        let mut before = state(7, 10, 84);
        before.active_rooms.insert(1);
        before.gold_piles.insert(9, 84);
        before.rooms.insert(9, 1);
        before.items.insert(10, (3, 2));
        before.rooms.insert(10, 1);
        ledger.observe(before, Events::default());
        // The player left room 1: its pile and item are freed with it.
        let mut after = state(7, 10, 0);
        after.active_rooms.insert(2);
        assert!(ledger.observe(after, Events::default()).is_empty());

        // The same pile vanishing from a room that is still active is a loss.
        let mut ledger = Conservation::default();
        let mut before = state(7, 10, 84);
        before.active_rooms.insert(1);
        before.gold_piles.insert(9, 84);
        before.rooms.insert(9, 1);
        ledger.observe(before, Events::default());
        let mut after = state(7, 10, 0);
        after.active_rooms.insert(1);
        let failures = ledger.observe(after, Events::default());
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].sig, "conservation:gold");
    }
    #[test]
    fn a_rooms_restored_ground_units_are_a_source() {
        let mut ledger = Conservation::default();
        let mut before = state(7, 10, 0);
        before.active_rooms.insert(2);
        before.player_rooms.insert(2);
        ledger.observe(before, Events::default());
        // Room 2 was active, and its remembered units come back live.
        let mut after = state(7, 10, 84);
        after.active_rooms.insert(2);
        after.player_rooms.insert(2);
        after.live_rooms.insert(2);
        after.gold_piles.insert(9, 84);
        after.gold_units.insert((UnitType::Item as u8, 9));
        after.rooms.insert(9, 2);
        assert!(ledger.observe(after.clone(), Events::default()).is_empty());

        // A unit whose room link lands one observation after the load is
        // still level population (an object's init can create its contents
        // late).
        let mut again = after.clone();
        again.gold = 120;
        again.gold_piles.insert(10, 36);
        again.rooms.insert(10, 2);
        assert!(ledger.observe(again.clone(), Events::default()).is_empty());

        // The load grace is over: a later pile needs a source again.
        pass_grace(&mut ledger, again.clone());
        let mut later = again;
        later.gold = 150;
        later.gold_piles.insert(11, 30);
        later.rooms.insert(11, 2);
        let failures = ledger.observe(later, Events::default());
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].sig, "conservation:gold");
    }

    #[test]
    fn a_pile_in_the_players_room_after_its_load_is_population() {
        let mut ledger = Conservation::default();
        let mut before = state(7, 10, 0);
        before.active_rooms.insert(2);
        before.player_rooms.insert(2);
        ledger.observe(before, Events::default());
        // The room loads; the player stands in it.
        let mut loaded = state(7, 10, 0);
        loaded.active_rooms.insert(2);
        loaded.live_rooms.insert(2);
        loaded.player_rooms.insert(2);
        ledger.observe(loaded, Events::default());
        // The level's pile arrives one observation later, in the player's
        // room and with no new monster to spot the load.
        let mut after = state(7, 10, 84);
        after.active_rooms.insert(2);
        after.live_rooms.insert(2);
        after.player_rooms.insert(2);
        after.gold_piles.insert(9, 84);
        after.gold_units.insert((UnitType::Item as u8, 9));
        after.rooms.insert(9, 2);
        assert!(ledger.observe(after.clone(), Events::default()).is_empty());

        pass_grace(&mut ledger, after.clone());
        let mut later = after;
        later.gold = 120;
        later.gold_piles.insert(10, 36);
        later.rooms.insert(10, 2);
        assert_eq!(ledger.observe(later, Events::default()).len(), 1);
    }

    #[test]
    fn a_pile_in_a_room_the_player_just_entered_is_population() {
        let mut ledger = Conservation::default();
        let mut before = state(7, 10, 0);
        before.active_rooms.insert(1);
        before.live_rooms.insert(1);
        before.player_rooms.insert(1);
        ledger.observe(before, Events::default());
        // The player walks into the already-live room 2.
        let mut entered = state(7, 10, 84);
        entered.active_rooms = [1, 2].into();
        entered.live_rooms = [1, 2].into();
        entered.player_rooms.insert(2);
        entered.gold_piles.insert(9, 84);
        entered.gold_units.insert((UnitType::Item as u8, 9));
        entered.rooms.insert(9, 2);
        assert!(ledger
            .observe(entered.clone(), Events::default())
            .is_empty());
        // Its pile may land in the next observation too.
        let mut later = entered;
        later.gold = 120;
        later.gold_piles.insert(10, 36);
        later.rooms.insert(10, 2);
        assert!(ledger.observe(later.clone(), Events::default()).is_empty());
        // The entry grace is over: a later pile needs a source again.
        pass_grace(&mut ledger, later.clone());
        let mut after = later;
        after.gold = 150;
        after.gold_piles.insert(11, 30);
        after.rooms.insert(11, 2);
        assert_eq!(ledger.observe(after, Events::default()).len(), 1);
    }

    #[test]
    fn a_no_room_item_with_a_level_load_is_population() {
        let mut ledger = Conservation::default();
        let mut before = state(7, 10, 0);
        before.active_rooms.insert(2);
        before.player_rooms.insert(2);
        ledger.observe(before, Events::default());
        // The room loads and an item whose room link has not landed yet
        // appears with it.
        let mut after = state(7, 10, 0);
        after.active_rooms.insert(2);
        after.live_rooms.insert(2);
        after.player_rooms.insert(2);
        after.items.insert(8, (68, 1));
        assert!(ledger.observe(after, Events::default()).is_empty());

        // Without a level load a new no-room item still needs a source.
        let mut ledger = Conservation::default();
        ledger.observe(state(7, 10, 0), Events::default());
        let mut after = state(7, 10, 0);
        after.items.insert(8, (68, 1));
        let failures = ledger.observe(after, Events::default());
        assert_eq!(failures.len(), 1);
        assert!(failures[0].sig.starts_with("conservation:items:68"));
    }
}
