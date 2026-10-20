//! The quest items used from the grid (C→S 0x20: `ass`, the Book of Skill,
//! and `xyz`, the Potion of Life) on the wired host with the play
//! preview's inventory rest (task `q-act3-act5-gaps`, REC-246): the
//! player's quest flag (9, 5) / (20, 5) gates the use, the use clears it,
//! adds the skill point / life and consumes the item. Made-up data.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use d2_server::adapters::character::StartItemWorld;
use d2_server::adapters::handlers::items::moves::{InvParts, MoveCall};
use d2_server::adapters::handlers::world::{
    preview_inv_parts, ActionEvents, ActionWorld, WiredStart, WiredWorld, WorldHost,
};
use d2_sim::game::Game;
use d2_sim::items::inventory::InvTables;
use d2_sim::items::moves::{self as sim_moves, MoveFatal, Owner};
use d2_sim::items::ItemTables;
use d2_sim::rng::Seed;
use d2_sim::units::lifecycle::{AllocRequest, LifecycleHooks};
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim};
use d2_sim::wiring::economy::{Economy, GameFields};
use d2_sim::world::npc::NpcControl;
use d2_sim::world::quests::{PlayerQuests, QuestControl, QuestTables};
use d2_sim::world::vendors::VendorTables;
use test_fixtures::game::{ActCreation, GameData, Seams};
use test_fixtures::{install, synth};

#[path = "../../d2-client/tests/e2e_support/mod.rs"]
mod e2e_support;
use e2e_support::Rest;

const TOWN: u32 = 1;
const INIT: u32 = 0x1234_5678;
const GAME_SEED: u32 = 1234;
const CLASS: u32 = 3;
const STAT_NEWSKILLS: u16 = 5;
const STAT_MAXHP: u16 = 7;

/// The synthetic set with the two quest items' `misc` rows.
fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("quest-item-use-{}", std::process::id()));
        let mut s = synth::synthetic();
        for code in ["ass", "xyz"] {
            s.tables.row(
                "misc",
                &[
                    ("code", code),
                    ("namestr", "pt1"),
                    ("type", "potn"),
                    ("level", "1"),
                    ("invwidth", "1"),
                    ("invheight", "1"),
                    ("useable", "1"),
                    ("cost", "5"),
                ],
            );
        }
        let i = install::build(&dir, &s).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

struct Fx {
    game: Game,
    sim: ActionSim<Seams>,
    world: WiredWorld<Rest>,
    player: UnitId,
}

/// A level-1 player (no room, as the save loader leaves it) on a wired
/// host with the install's item and inventory tables; `inventory`: the
/// host has an inventory model.
fn fx() -> Fx {
    let d = data();
    let (drlg, types) = d.level_types();
    let drlg = d
        .drlg_world(drlg, &types, ActCreation::TownOnly, INIT, TOWN)
        .unwrap();
    let mut hooks = ActionHooks::new(
        Arc::new(d.action_tables().unwrap()),
        drlg,
        Seed::init_low(GAME_SEED),
        Seams::default(),
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
    let _ = guid;
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
    world.inventory = Some(preview_inv_parts(InvTables::from_fixed(&d.fixed).unwrap()));
    world.rest.quests.insert(player, PlayerQuests::default());
    Fx {
        game,
        sim,
        world,
        player,
    }
}

impl Fx {}

/// Makes `code` for the player and puts it in the inventory grid; its GUID.
fn give(f: &mut Fx, code: [u8; 4]) -> (UnitId, u32) {
    let player = f.player;
    let item = f.world.with_economy(&mut f.game, &mut f.sim, |econ, p| {
        let inv = p.inventory.as_deref_mut().expect("inventory model");
        let guid = econ.units.get(player).unwrap().guid;
        if !inv.state.inventories.contains_key(&player) {
            inv.state.add_inventory(
                player,
                d2_sim::items::inventory::UnitKind::Player { class: CLASS as u8 },
                guid,
            );
        }
        let mut w = WiredStart {
            econ,
            inv,
            player,
            owner: Owner::player(guid),
            faults: Vec::new(),
        };
        let it = w.create(code).expect("the item");
        assert!(w.place_inventory(it), "placed");
        assert!(w.faults.is_empty(), "{:?}", w.faults);
        it
    });
    let guid = f.sim.sys.units.get(item).unwrap().guid;
    (item, guid)
}

/// C→S 0x20 UseGridItem through the move handler of the wired host.
struct Use {
    player: UnitId,
    msg: Vec<u8>,
}

impl MoveCall for Use {
    type Out = Option<Result<u32, MoveFatal>>;
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        let mut d = parts.desk(econ);
        let guid = d.guid_of(self.player);
        sim_moves::handle(&mut d, guid, &self.msg)
    }
}

fn use_grid(f: &mut Fx, guid: u32) {
    let mut msg = vec![0x20];
    msg.extend_from_slice(&guid.to_le_bytes());
    msg.extend_from_slice(&0u32.to_le_bytes());
    msg.extend_from_slice(&0u32.to_le_bytes());
    let player = f.player;
    let r = f
        .world
        .moves(&mut f.game, &mut f.sim, Use { player, msg })
        .expect("the inventory model");
    assert!(matches!(r, Some(Ok(_))), "{r:?}");
}

fn flag(f: &mut Fx, q: u8, b: u8) -> bool {
    f.world.rest.quests[&f.player].flags[0].get(q, b)
}

fn stat(f: &mut Fx, s: u16) -> i32 {
    f.sim.sys.stats.unit_total(f.player, s, 0)
}

fn in_grid(f: &Fx, item: UnitId) -> bool {
    f.world.inventory.as_ref().unwrap().state.inventories[&f.player]
        .items()
        .contains(&item)
}

#[test]
fn the_book_of_skill_adds_a_skill_point_and_is_used_up() {
    let mut f = fx();
    let (item, guid) = give(&mut f, *b"ass ");
    // Without the quest flag (9, 5) the book stays (sound only).
    let before = stat(&mut f, STAT_NEWSKILLS);
    use_grid(&mut f, guid);
    assert_eq!(stat(&mut f, STAT_NEWSKILLS), before);
    assert!(in_grid(&f, item), "the book stays");
    // With it: the flag clears, +1 skill point, the item is consumed.
    f.world.rest.quests.get_mut(&f.player).unwrap().flags[0].set(9, 5);
    use_grid(&mut f, guid);
    assert!(!flag(&mut f, 9, 5), "the flag is cleared");
    assert_eq!(stat(&mut f, STAT_NEWSKILLS), before + 1);
    assert!(!in_grid(&f, item), "the book is used up");
}

#[test]
fn the_potion_of_life_adds_twenty_life_and_is_used_up() {
    let mut f = fx();
    let (item, guid) = give(&mut f, *b"xyz ");
    let before = stat(&mut f, STAT_MAXHP);
    use_grid(&mut f, guid);
    assert_eq!(stat(&mut f, STAT_MAXHP), before);
    assert!(in_grid(&f, item));
    f.world.rest.quests.get_mut(&f.player).unwrap().flags[0].set(20, 5);
    use_grid(&mut f, guid);
    assert!(!flag(&mut f, 20, 5));
    assert_eq!(stat(&mut f, STAT_MAXHP), before + 0x1400);
    assert!(!in_grid(&f, item));
}
