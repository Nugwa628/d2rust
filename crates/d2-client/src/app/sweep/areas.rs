//! Area routes, loaded waypoint destinations, and explicit level links.
use super::*;
use d2_data::tables::{Leveldefs, Monstats};
use d2_sim::world::{vendors::TOWNS, waypoints::WaypointMap};

/// Links whose exit is a portal object, not a level warp tile, so there is no
/// tile for the link case to walk to: the Throne of Destruction and the
/// Worldstone Chamber are joined only by Baal's portal (a full exploration of
/// either level finds no tile to the other), and level 133 has no warp tile at
/// all. Counted as a skip.
const PORTAL_EXITS: [(usize, u32); 3] = [(131, 132), (132, 131), (133, 17)];

pub(super) fn generate(live: &LiveData, g: &mut Generated) -> Result<()> {
    let levels = live.tables.rows::<Leveldefs>()?;
    let valid = |id: usize| {
        id > 0
            && levels.get(id).is_some_and(|r| r.drlgtype > 0)
            && live.waypoints.levels.get(id).is_some_and(|l| l.act < 5)
    };
    for (id, row) in levels.iter().enumerate() {
        if !valid(id) {
            g.skip("areas:no-playable-generator-or-act");
            continue;
        }
        let act = live.waypoints.levels[id].act;
        let header = format!("char class 0\nchar area {act} {id}");
        g.add("areas", format!("area-{id}-walk"), &header,
            &format!("at 0 dev god on\nat 4 msg Walk x=@x+4 y=@y\nat 24 msg Walk x=@x-4 y=@y\nat 44 msg Walk x=@x y=@y+4\nat 64 msg Walk x=@x y=@y-4\nby 88 check player.pos changes\nat 92 check player.area == {id}"), 96)?;
        let destinations = [
            row.vis0, row.vis1, row.vis2, row.vis3, row.vis4, row.vis5, row.vis6, row.vis7,
        ];
        let warps = [
            row.warp0, row.warp1, row.warp2, row.warp3, row.warp4, row.warp5, row.warp6, row.warp7,
        ];
        for (slot, (destination, warp)) in destinations.into_iter().zip(warps).enumerate() {
            if destination == 0 {
                continue;
            }
            if !valid(destination as usize) {
                g.skip("areas:link-destination-not-playable");
                continue;
            }
            if PORTAL_EXITS.contains(&(id, destination)) {
                g.skip("areas:portal-exit-without-tile");
                continue;
            }
            if warp == u32::MAX {
                g.skip("areas:seam-link-without-interaction-tile");
                continue;
            }
            // The summit's exits to 118 / 128 wait for the Ancients (A5Q5):
            // mark the quest completed before the join so the warp gate is
            // open.
            let mut header = header.clone();
            if id == 120 && matches!(destination, 118 | 128) {
                header.push_str("\nchar quest 39 0x8000");
            }
            // Durance of Hate 1 waits for the Compelling Orb (quest slot 18's
            // reward, which game start reads as "orb smashed"), except from
            // Durance 2.
            if destination == 100 {
                header.push_str("\nchar quest 18 0x0001");
            }
            // `to`: a lvlwarp class may be shared by two links of the level;
            // keep only the tile whose link leads to this destination.
            // `@near`: interact with that tile, the one next to the player,
            // not the first unit of the class.
            g.add("areas", format!("link-{id}-{slot}-to-{destination}"), &header,
                &format!("at 0 dev god on\nat 1 poke goto preset {id} 5:{warp} to {destination}\nat 420 msg InteractWithEntity type=5 id=@5@near\nby 480 check player.area == {destination}"), 484)?;
        }
    }
    for (_, destination, act, _, _) in WaypointMap::new(&live.waypoints.levels).rows() {
        let town = TOWNS[usize::from(act.min(4))];
        if act >= 5 || !valid(destination as usize) || !valid(town as usize) {
            g.skip("areas:waypoint-missing-area-or-town");
            continue;
        }
        let object = live.waypoints.object_class;
        g.add("areas", format!("waypoint-to-{destination}"),
            &format!("char class 0\nchar area {act} {town}"),
            &format!("at 0 dev god on\nat 0 dev waypoints\nat 1 poke object {object} @x+3 @y\nat 2 poke goto unit 2:{object}\nat 4 msg InteractWithEntity type=2 id=@2:{object}\nat 12 msg TakeOrCloseWp wp=@2:{object} level={destination}\nby 60 check player.area == {destination}"), 64)?;
    }
    let monsters = live.tables.rows::<Monstats>()?;
    let attacker = monsters
        .iter()
        .position(|m| m.enabled && !m.npc && m.killable && m.a1maxd > 0);
    for act in 0..5 {
        let area = (1..levels.len()).find(|&id| {
            valid(id) && live.waypoints.levels[id].act == act && !TOWNS.contains(&(id as u16))
        });
        let (Some(area), Some(attacker)) = (area, attacker) else {
            g.skip("areas:death-needs-combat-area-and-attacker");
            continue;
        };
        let town = TOWNS[usize::from(act)];
        g.add("areas", format!("act-{act}-die-respawn"),
            &format!("char class 0\nchar area {act} {area}\nchar stat 6 256\nchar stat 7 256"),
            &format!("at 0 spawn {attacker} @x+3 @y normal free\nby 300 check player.mode == dead\nat 304 msg Resurrect\nby 360 check player.area == {town}\nat 360 check player.life > 0"), 364)?;
    }
    Ok(())
}
