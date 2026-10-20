//! The start items of a new character on the wired host
//! (`WiredWorld::start_items`) over the synthetic install: its charstats
//! give every class `sb1` (a `blad`, body location `rarm`) once and `pt1`
//! (a `potn`, not beltable here, location 0) twice.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use d2_server::adapters::character::StartPlace;
use d2_server::adapters::handlers::world::{ActionEvents, ActionWorld, WiredWorld};
use d2_sim::game::Game;
use d2_sim::items::inventory::InvTables;
use d2_sim::items::{flag, ItemTables};
use d2_sim::rng::Seed;
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, NoPending};
use d2_sim::wiring::economy::GameFields;
use d2_sim::world::npc::NpcControl;
use d2_sim::world::quests::{QuestControl, QuestTables};
use d2_sim::world::vendors::VendorTables;
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::{install, synth};

#[path = "../../d2-client/tests/e2e_support/mod.rs"]
mod e2e_support;
use e2e_support::{inv_parts, InvFx, Rest};

const TOWN: u32 = 1;
const INIT: u32 = 0x1234_5678;
const GAME_SEED: u32 = 1234;
const CLASS: u32 = 3;
/// `bodylocs` row of `rarm` (the synthetic table's order).
const RARM: u8 = 4;

fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("start-items-{}", std::process::id()));
        let i = install::build(&dir, &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

struct Fx {
    game: Game,
    sim: ActionSim<NoPending>,
    world: WiredWorld<Rest>,
    player: UnitId,
}

/// A level-1 player (no room, as the save loader leaves it) on a wired
/// host with the install's item and inventory tables; `inventory`: the
/// host has an inventory model.
fn fx(inventory: bool, add_player_inventory: bool) -> Fx {
    fx_on(data(), inventory, add_player_inventory)
}

/// [`data`] plus the made-up Horadric Cube item (`test_fixtures::cube_item`).
fn data_with_cube() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("start-items-cube-{}", std::process::id()));
        let mut s = synth::synthetic();
        test_fixtures::cube_item::add_cube_item(&mut s.tables);
        let i = install::build(&dir, &s).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

fn fx_on(d: &'static GameData, inventory: bool, add_player_inventory: bool) -> Fx {
    let (drlg, types) = d.level_types();
    let drlg = d
        .drlg_world(drlg, &types, ActCreation::TownOnly, INIT, TOWN)
        .unwrap();
    let mut hooks = ActionHooks::new(
        Arc::new(d.action_tables().unwrap()),
        drlg,
        Seed::init_low(GAME_SEED),
        NoPending,
    );
    hooks.vitals = Some(Arc::new(d.vitals().unwrap()));
    let mut sim = ActionSim::new(
        Arc::new(d.stat_data().unwrap()),
        d.unit_data().unwrap(),
        hooks,
    );
    sim.create_game(&GameFields::new(Seed::init_low(GAME_SEED), true));
    let mut game = Game::new();
    game.lists.ensure_act(0).unwrap();
    let req = AllocRequest {
        ty: UnitType::Player,
        class: CLASS,
        room: None,
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: true,
    };
    let player = sim
        .with(&mut game, |g, v| v.allocate(g, &req, 0, 0))
        .unwrap();
    let sys = &mut sim.sys;
    sys.stats.unit_set(&mut sys.hooks, player, 12, 1, 0);
    let guid = sys.units.get(player).unwrap().guid;
    let items = ItemTables::from_fixed(&d.fixed).unwrap();
    let mut seed = sim.hooks().game_seed;
    let npc = NpcControl::new(&[], Vec::new(), true, 0, &mut seed).unwrap();
    let quests = QuestControl::new(&QuestTables::load().unwrap(), &mut seed).unwrap();
    sim.hooks().game_seed = seed;
    let mut world = WiredWorld::new(
        ActionWorld::default(),
        items,
        quests,
        npc,
        VendorTables::default(),
        Rest::default(),
        1000,
    );
    if inventory {
        let t = InvTables::from_fixed(&d.fixed).unwrap();
        let mut parts = inv_parts(t, InvFx::default(), player, CLASS as u8, guid);
        if !add_player_inventory {
            parts.state.inventories.clear();
        }
        world.inventory = Some(parts);
    }
    Fx {
        game,
        sim,
        world,
        player,
    }
}

impl Fx {
    fn code(&mut self, item: UnitId) -> [u8; 4] {
        let rec = self.sim.hooks().items.get(item).unwrap().record;
        self.world.tables.items[rec].code
    }
}

#[test]
fn a_new_character_gets_its_charstats_items_in_slot_order() {
    let mut f = fx(true, true);
    let seed = f.sim.hooks().game_seed;
    let r = f.world.start_items(&mut f.game, &mut f.sim, f.player);
    assert!(r.faults.is_empty(), "{:?}", r.faults);
    let places: Vec<StartPlace> = r.items.iter().map(|i| i.1).collect();
    assert_eq!(
        places,
        [
            StartPlace::Equipped(RARM),
            StartPlace::Inventory,
            StartPlace::Inventory
        ]
    );
    let codes: Vec<[u8; 4]> = r
        .items
        .iter()
        .map(|i| i.0)
        .collect::<Vec<_>>()
        .into_iter()
        .map(|u| f.code(u))
        .collect();
    assert_eq!(codes, [*b"sb1 ", *b"pt1 ", *b"pt1 "]);
    for &(item, _) in &r.items {
        let it = f.sim.hooks().items.get(item).unwrap();
        // A start item, identified.
        assert_ne!(it.flags & flag::STARTITEM, 0);
        assert_ne!(it.flags & flag::IDENTIFIED, 0);
    }
    // Each copy is an item creation: the game seed moved.
    assert_ne!(f.sim.hooks().game_seed, seed);
    // The weapon's durability is its max (step 2.6).
    let sb1 = r.items[0].0;
    let s = &f.sim.sys.stats;
    assert_eq!(s.unit_total(sb1, 72, 0), s.unit_total(sb1, 73, 0));
    // Equipped in the inventory model; the potions on page 0.
    let inv = f.world.inventory.as_ref().unwrap();
    let pinv = &inv.state.inventories[&f.player];
    assert!(pinv.items().contains(&sb1));
    assert_eq!(inv.state.items[&sb1].body_loc, RARM);
    for &(p, _) in &r.items[1..] {
        assert!(pinv.items().contains(&p));
        assert_eq!(inv.state.items[&p].page, 0);
    }
}

#[test]
fn the_player_inventory_is_made_when_missing_and_no_model_is_a_fault() {
    let mut f = fx(true, false);
    let r = f.world.start_items(&mut f.game, &mut f.sim, f.player);
    assert_eq!(r.items.len(), 3, "{:?}", r.faults);
    assert!(f
        .world
        .inventory
        .as_ref()
        .unwrap()
        .state
        .inventories
        .contains_key(&f.player));
    let mut f = fx(false, false);
    let seed = f.sim.hooks().game_seed;
    let r = f.world.start_items(&mut f.game, &mut f.sim, f.player);
    assert!(r.items.is_empty());
    assert_eq!(r.faults.len(), 1);
    // Nothing was created: no draw.
    assert_eq!(f.sim.hooks().game_seed, seed);
}

/// Determinism: the same game gives the same items.
#[test]
fn start_items_are_deterministic() {
    let run = || {
        let mut f = fx(true, true);
        let r = f.world.start_items(&mut f.game, &mut f.sim, f.player);
        let seeds: Vec<_> = r
            .items
            .iter()
            .map(|&(i, p)| (f.sim.hooks().items.get(i).unwrap().init_seed, p))
            .collect();
        (seeds, f.sim.hooks().game_seed)
    };
    assert_eq!(run(), run());
}

#[test]
fn the_new_character_load_makes_start_items_on_the_preview_inventory() {
    use d2_server::adapters::handlers::world::preview_inv_parts;
    use d2_server::adapters::session::load_new_character_with_items;
    use d2_server::adapters::SimGame;
    let Fx {
        game,
        sim,
        mut world,
        player,
    } = fx(false, false);
    world.inventory = Some(preview_inv_parts(
        InvTables::from_fixed(&data().fixed).unwrap(),
    ));
    let mut s = SimGame::with_world(game, sim, world);
    let (_, report, items) = load_new_character_with_items(&mut s, player, [0; 16]);
    assert!(items.faults.is_empty(), "{:?}", items.faults);
    assert_eq!(items.items.len(), 3);
    assert_eq!(items.items[0].1, StartPlace::Equipped(RARM));
    let steps: Vec<_> = report.unapplied.iter().map(|u| u.step).collect();
    assert!(!steps.contains(&"start items"), "{steps:?}");
    // Without an inventory model the step stays unapplied.
    let Fx {
        game,
        sim,
        world,
        player,
    } = fx(false, false);
    let mut s = SimGame::with_world(game, sim, world);
    let (_, report, items) = load_new_character_with_items(&mut s, player, [0; 16]);
    assert_eq!(items.faults.len(), 1);
    assert!(report.unapplied.iter().any(|u| u.step == "start items"));
}

#[test]
fn a_new_character_has_the_cube_when_the_host_names_it_as_an_extra() {
    let mut f = fx_on(data_with_cube(), true, true);
    f.world.start_extra = vec![*b"box "];
    let r = f.world.start_items(&mut f.game, &mut f.sim, f.player);
    assert!(r.faults.is_empty(), "{:?}", r.faults);
    // The charstats items first, the cube after them, in the inventory.
    let codes: Vec<[u8; 4]> = r
        .items
        .iter()
        .map(|i| i.0)
        .collect::<Vec<_>>()
        .into_iter()
        .map(|u| f.code(u))
        .collect();
    assert_eq!(codes, [*b"sb1 ", *b"pt1 ", *b"pt1 ", *b"box "]);
    assert_eq!(r.items[3].1, StartPlace::Inventory);
    // Without the extra, the class's three start items only.
    let mut f = fx_on(data_with_cube(), true, true);
    let r = f.world.start_items(&mut f.game, &mut f.sim, f.player);
    assert_eq!(r.items.len(), 3);
}
