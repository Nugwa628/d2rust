//! [`WorldSim::create_game`]: the four game-seed derivations in order.

use std::sync::Arc;

use super::*;
use crate::wiring::worldgen::CreationTables;
use crate::world::npc::NpcControl;
use crate::world::objects::ObjectTables;
use crate::world::quests::{QuestControl, QuestTables};

// The creation step's draw order: regions, objects, the NPC seed, quests.
#[test]
fn creation_derives_regions_objects_npc_and_quests_in_order() {
    let qt = QuestTables::load().unwrap();
    let mut a = Fx::new(isle_ds1s());
    let mut b = Fx::new(isle_ds1s());
    let start = a.sim.action.sys.hooks.game_seed;
    assert_eq!(start, b.sim.action.sys.hooks.game_seed);

    let made = a
        .sim
        .create_game(CreationTables {
            objects: Arc::new(ObjectTables::default()),
            monstats: &[],
            hirelings: Vec::new(),
            quests: &qt,
        })
        .unwrap();

    // The same four steps by hand.
    b.sim.create_regions();
    b.sim.create_objects(Arc::new(ObjectTables::default()));
    let mut seed = b.sim.action.sys.hooks.game_seed;
    let npc = NpcControl::new(&[], Vec::new(), false, 0, &mut seed).unwrap();
    let quests = QuestControl::new(&qt, &mut seed).unwrap();

    assert_eq!(a.sim.action.sys.hooks.game_seed, seed);
    assert_eq!(made.npc.seed, npc.seed);
    assert_eq!(made.quests.seed, quests.seed);
    assert!(a.sim.action.sys.hooks.objects.is_some());
    // Each control has its own seed: four steps from the start.
    let mut s = start;
    for _ in 0..4 {
        s.step();
    }
    assert_eq!(a.sim.action.sys.hooks.game_seed, s);
    // Dropping the object step shifts the NPC seed.
    let mut c = Fx::new(isle_ds1s());
    c.sim.create_regions();
    let mut seed = c.sim.action.sys.hooks.game_seed;
    let npc_no_objects = NpcControl::new(&[], Vec::new(), false, 0, &mut seed).unwrap();
    assert_ne!(npc_no_objects.seed, made.npc.seed);
}
