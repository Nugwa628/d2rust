//! The preview rest's skill seams over the players' skill lists (task
//! `q-preview-skill-seams`, REC-266): wearing a throwing weapon selects
//! its throw skill, and a worn item's `item_singleskill` adds an entry to
//! the list (and the client's 0x21) that leaves with the item. Made-up data.

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
use d2_sim::items::{ItemStats, ItemTables};
use d2_sim::rng::Seed;
use d2_sim::skills::list::SkillList;
use d2_sim::units::lifecycle::{AllocRequest, LifecycleHooks};
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, ActionSim, ActionTables};
use d2_sim::wiring::economy::{Economy, GameFields, UnitStats};
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
/// Throw (`SKILL_THROW`) is skills row 2 and the singleskill is row 3
/// (class 3 here): the synthetic rows are renamed in meaning below.
const THROW: i32 = 2;
const WISP: i32 = 3;
const STAT_SINGLESKILL: u16 = 107;
/// Item type rows the bookkeeping hard-codes: 45 `weap`, 46 `mele`, 48 `thro`.
const THRO: usize = 48;

fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("preview-skill-seams-{}", std::process::id()));
        let mut s = synth::synthetic();
        // Pad the item types so `twpn` is row 45, `tmel` 46, `thro` 48.
        let mut n = s.tables.file("itemtypes.txt").rows.len();
        let add = |t: &mut synth::TableSet, code: &str, equiv: &str, body: &str| {
            t.row(
                "itemtypes",
                &[
                    ("code", code),
                    ("equiv1", equiv),
                    ("storepage", "weap"),
                    ("body", if body.is_empty() { "0" } else { "1" }),
                    ("bodyloc1", body),
                    ("normal", "1"),
                    // itemtypes `Throwable`, as the game files'
                    // `thro` and `jave` rows.
                    (
                        "throwable",
                        if matches!(code, "thro" | "jave") {
                            "1"
                        } else {
                            "0"
                        },
                    ),
                ],
            );
        };
        while n < 45 {
            add(&mut s.tables, &format!("fl{n}"), "", "");
            n += 1;
        }
        add(&mut s.tables, "twpn", "", ""); // 45
        add(&mut s.tables, "tmel", "twpn", ""); // 46
        add(&mut s.tables, "fl47", "", ""); // 47
        add(&mut s.tables, "thro", "twpn", ""); // 48
        add(&mut s.tables, "jave", "thro", "rarm"); // 49
        s.tables.row(
            "weapons",
            &[
                ("code", "jv1"),
                ("namestr", "jv1"),
                ("type", "jave"),
                ("level", "1"),
                ("levelreq", "1"),
                ("mindam", "1"),
                ("maxdam", "4"),
                ("invwidth", "1"),
                ("invheight", "3"),
                ("durability", "30"),
                ("cost", "40"),
                ("normcode", "jv1"),
                ("wclass", "1hs"),
                ("2handedwclass", "1hs"),
                ("stackable", "0"),
            ],
        );
        // Pad the stats so `item_singleskill` is stat 107.
        let mut n = s.tables.file("itemstatcost.txt").rows.len();
        while n <= STAT_SINGLESKILL as usize {
            let name = if n == STAT_SINGLESKILL as usize {
                "item_singleskill".to_string()
            } else {
                format!("fillstat{n}")
            };
            // `fCallback` 1 as in the game files: the stat 107 callback
            // runs on its changes.
            let fcb = if n == STAT_SINGLESKILL as usize {
                "1"
            } else {
                "0"
            };
            s.tables.row(
                "itemstatcost",
                &[
                    ("stat", &name),
                    ("fcallback", fcb),
                    ("send bits", "10"),
                    ("save bits", "10"),
                    ("csvbits", "10"),
                    ("descpriority", &(n * 3).to_string()),
                ],
            );
            n += 1;
        }
        let i = install::build(&dir, &s).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

/// The action tables with row 2 made the Throw skill (`itypea1` `thro`,
/// `range` 2) and row 3 a skill of class 3.
fn tables(d: &GameData) -> ActionTables {
    let mut t = d.action_tables().unwrap();
    t.skills.skills[THROW as usize].itypea1 = THRO as u16;
    t.skills.skills[THROW as usize].range = 2;
    t.skills.skills[WISP as usize].charclass = CLASS as u8;
    t
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
        Arc::new(tables(d)),
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
    // Strength and dexterity for any requirement (stats 0 and 2).
    sys.stats.unit_set(&mut sys.hooks, player, 0, 100, 0);
    sys.stats.unit_set(&mut sys.hooks, player, 2, 100, 0);
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
    let mut inv = preview_inv_parts(InvTables::from_fixed(&d.fixed).unwrap());
    // The synthetic item types have no class cell (255); "no restriction"
    // is 7 (`equip::CLASS_NONE`).
    for r in &mut inv.tables.itemtypes {
        r.class = 7;
    }
    world.inventory = Some(inv);
    world.rest.quests.insert(player, PlayerQuests::default());
    // The player's list: Attack, Throw and the class skill, Attack on
    // both hands.
    let rows = &sim.hooks().tables.skills.skills;
    let mut list = SkillList::default();
    let lo = d2_sim::skills::list::ListOwner::player(CLASS as i32);
    for s in [0, THROW] {
        list.add(rows, lo, s);
    }
    list.select(rows, true, 0, -1).unwrap();
    list.select(rows, false, 0, -1).unwrap();
    sim.hooks().skill_lists.insert(player, list);
    Fx {
        game,
        sim,
        world,
        player,
    }
}

impl Fx {}

/// Makes `code` for the player and puts it in the inventory grid; its GUID.
fn give(f: &mut Fx, code: [u8; 4], stat107: Option<i32>) -> (UnitId, u32) {
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
        // Worn items must be identified.
        w.econ.items.get_mut(it).unwrap().flags |= d2_sim::items::flag::IDENTIFIED;
        assert!(w.faults.is_empty(), "{:?}", w.faults);
        if let Some(skill) = stat107 {
            w.econ.with_stats(|ctx| {
                UnitStats::new(ctx, it).set_base(STAT_SINGLESKILL, skill as u16, 1)
            });
        }
        it
    });
    let guid = f.sim.sys.units.get(item).unwrap().guid;
    (item, guid)
}

/// One item-move message, flushed as the server's handler does.
struct Msg {
    player: UnitId,
    msg: Vec<u8>,
}

impl MoveCall for Msg {
    type Out = (Option<Result<u32, MoveFatal>>, Vec<Vec<u8>>);
    fn call<H: LifecycleHooks>(self, econ: &mut Economy<'_, H>, parts: &mut InvParts) -> Self::Out {
        let mut d = parts.desk(econ);
        let guid = d.guid_of(self.player);
        let r = sim_moves::handle(&mut d, guid, &self.msg);
        d.flush_equip();
        let sent = d.rest.take_sent().into_iter().map(|(_, b)| b).collect();
        (r, sent)
    }
}

fn send(f: &mut Fx, bytes: Vec<u8>) -> Vec<Vec<u8>> {
    let player = f.player;
    let (r, mut sent) = f
        .world
        .moves(&mut f.game, &mut f.sim, Msg { player, msg: bytes })
        .expect("the inventory model");
    assert!(matches!(r, Some(Ok(_))), "{r:?}");
    // What the call made after its own take (the synced skill entries).
    sent.extend(
        f.world
            .inventory
            .as_mut()
            .unwrap()
            .rest
            .take_sent()
            .into_iter()
            .map(|(_, b)| b),
    );
    sent
}

/// 0x19 (grid to cursor) then 0x1A (cursor to body location `loc`).
fn wear(f: &mut Fx, guid: u32, loc: u32) -> Vec<Vec<u8>> {
    let mut m = vec![0x19];
    m.extend_from_slice(&guid.to_le_bytes());
    send(f, m);
    let mut m = vec![0x1A];
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(&loc.to_le_bytes());
    send(f, m)
}

fn list(f: &Fx) -> &SkillList {
    &f.sim.sys.hooks.skill_lists[&f.player]
}

/// The (skill, owner) a side points at.
fn hand(f: &Fx, left: bool) -> (i32, i32) {
    let l = list(f);
    let e = l.entries[if left { l.left } else { l.right }.unwrap()];
    (i32::from(e.skill), e.owner)
}

#[test]
fn wearing_a_throwing_weapon_offers_its_throw_skill() {
    let mut f = fx();
    assert_eq!(hand(&f, true), (0, -1));
    let (_, guid) = give(&mut f, *b"jv1 ", None);
    let sent = wear(&mut f, guid, 4);
    assert_eq!(hand(&f, true), (THROW, -1), "Throw on the left");
    // The client is told (S→C 0x23, hand 1, skill 2).
    assert!(
        sent.iter()
            .any(|m| m.first() == Some(&0x23) && m[6] == 1 && m[7] == THROW as u8),
        "{sent:?}"
    );
}

#[test]
fn a_worn_singleskill_enters_the_list_and_leaves_with_the_item() {
    let mut f = fx();
    assert!(!list(&f).has(WISP));
    let (_, guid) = give(&mut f, *b"ar1 ", Some(WISP));
    let sent = wear(&mut f, guid, 3);
    let l = list(&f);
    let i = l.native(WISP).expect("the entry");
    assert_eq!(l.entries[i].base, 0);
    // The client's 0x21 (add, skill 3).
    assert!(
        sent.iter()
            .any(|m| m.first() == Some(&0x21) && m[2] == 0 && m[7] == WISP as u8),
        "{sent:?}"
    );
    // It can be put on a mouse slot (the list's select, as 0x3C does).
    let rows = f.sim.sys.hooks.tables.clone();
    let l = f.sim.sys.hooks.skill_lists.get_mut(&f.player).unwrap();
    l.select(&rows.skills.skills, false, WISP, -1).unwrap();
    assert_eq!(hand(&f, false), (WISP, -1));
    // Taking the item off removes the entry again; the hand is Attack.
    let mut m = vec![0x1C];
    m.extend_from_slice(&3u16.to_le_bytes());
    send(&mut f, m);
    assert!(!list(&f).has(WISP));
    assert_eq!(hand(&f, false), (0, -1));
}
