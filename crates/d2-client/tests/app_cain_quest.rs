//! The quest world calls Cain's quest needs, on the play preview's
//! built game on the user's install (`D2_GAME_DIR`): a monster spawned in a DRLG room
//! (Cain in town), a portal object to a level (the red portal to
//! Tristram), an object looked up by class and a monster removed.
//! Before `HostQuests` answered them they went to `AppRest`, which only
//! logged and returned nothing.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use d2_client::app::single_player;
use d2_client::bridge::link::{SendQueue, ServerLink};
use d2_server::adapters::handlers::world::{QuestCall, WorldHost};
use d2_server::seams::Clock;
use d2_sim::units::{RoomId, UnitId};
use d2_sim::world::npc::class;
use d2_sim::world::quests::{QuestControl, QuestWorld};

use crate::app_support;

const TRISTRAM: u32 = 38;
/// The install's `objects` row 59 (TownPortal; the red portal's class).
const PORTAL_CLASS: u16 = 59;

struct Probe {
    room: RoomId,
    at: (i32, i32),
}

#[derive(Debug)]
struct Out {
    cain: Option<UnitId>,
    cain_class: Option<u16>,
    portal: Option<UnitId>,
    found: Option<UnitId>,
    removed_gone: bool,
}

impl QuestCall for Probe {
    type Out = Out;
    fn call<W: QuestWorld>(self, _: &mut QuestControl, w: &mut W) -> Out {
        let (x, y) = self.at;
        // Spread 5, the first try of the quest's own spawn
        // (step 3): spread 0 fails the ring
        // search with no draws.
        let cain = w.spawn_monster(self.room, x, y, class::CAIN5, 1, 5);
        let cain_class = cain.and_then(|c| w.monster_class(c));
        let portal = w.open_portal(None, self.room, x + 6, y, TRISTRAM, PORTAL_CLASS, false);
        let found = cain.and_then(|c| w.find_object_near(c, PORTAL_CLASS));
        let removed_gone = match cain {
            Some(c) => {
                w.remove_monster(c);
                w.monster_class(c).is_none()
            }
            None => false,
        };
        Out {
            cain,
            cain_class,
            portal,
            found,
            removed_gone,
        }
    }
}

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn cains_spawn_the_red_portal_and_his_removal_run_on_real_units() {
    // The install's game with its player in the Rogue Encampment: the
    // join (C→S 0x67, 0x6B) and a few ticks, so the town's rooms are in
    // play (the build alone has no player and no active room).
    let ms = Arc::new(AtomicU32::new(1000));
    let (mut link, _) = single_player::start(
        app_support::game_data(),
        single_player::DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    let req = single_player::create_request();
    link.send(SendQueue::System, &req.encode()).unwrap();
    link.send(SendQueue::System, &[0x6B]).unwrap();
    for _ in 0..8 {
        link.pump().unwrap();
        ms.fetch_add(40, Ordering::SeqCst);
        link.pump().unwrap();
        link.receive();
    }
    link.with(|l| {
        let s = &mut l.host_mut().game;
        run(s);
    })
    .unwrap();
}

fn run(s: &mut d2_client::app::single_player::Sim) {
    // A free sub-tile of the Rogue Encampment's room in play with a free
    // one 6 east of it (the real town's collision, move mask 0x1C09).
    let (room, at) = {
        let rooms = s.game.lists.active_rooms(0);
        let h = s.events.action.hooks();
        let d = h.drlg.dungeon.acts[0].as_ref().expect("Act I");
        let free = |x: i32, y: i32| d.collision_at(x, y).is_some_and(|m| m & 0x1C09 == 0);
        rooms
            .into_iter()
            .filter(|&r| h.drlg.level_id(&s.game, r) == Some(single_player::ACT1_TOWN))
            .find_map(|r| {
                let t = h.drlg.subtiles(&s.game, r)?;
                (t.y..t.y + t.h)
                    .flat_map(|y| (t.x..t.x + t.w - 6).map(move |x| (x, y)))
                    .find(|&(x, y)| free(x, y) && free(x + 6, y))
                    .map(|p| (r, p))
            })
            .expect("a free spot in the town's room")
    };
    let probe = Probe { room, at };
    let out = WorldHost::quests(&mut s.world, &mut s.game, &mut s.events, probe).unwrap();
    assert!(out.cain.is_some(), "{out:?}");
    assert_eq!(out.cain_class, Some(class::CAIN5));
    let portal = out.portal.expect("the portal object");
    assert_eq!(out.found, Some(portal));
    assert!(out.removed_gone, "{out:?}");
    // Class 59's init sets `InteractType` to the town level of the
    // room's act (object init 11), here the Rogue
    // Encampment; the far end's destination is the requested level.
    let st = s.events.action.hooks().objects.as_ref().unwrap();
    assert_eq!(
        st.control.data.get(&portal).map(|d| u32::from(d.interact)),
        Some(single_player::ACT1_TOWN)
    );
    // Nothing fell through to the rest's log.
    assert!(
        s.world
            .rest
            .log
            .iter()
            .all(|l| !l.starts_with("spawn monster") && !l.starts_with("remove monster")),
        "{:?}",
        s.world.rest.log
    );
}
