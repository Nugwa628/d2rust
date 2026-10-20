//! Doors: every door class of every level that places doors, opened by a
//! click and walked through both ways.
//!
//! Stated rules (owner report E5: doors don't really open, walking through
//! them doesn't work): a closed door opens when the player clicks it, and
//! once open the player walks through it, to either side. Each case brings
//! the player to a closed door of the class in the level (`poke goto
//! preset LEVEL 2:C mode 0 opening`: the lowest closed door of the class,
//! the player landing a few sub-tiles in front of its opening on the
//! positive side of its narrow axis, `d2_sim::poke::door_opening`), clears the level's monsters and
//! those near the player (so clicks reach the door or the ground, not a
//! monster; the monsters of the next level come through an opened door, so
//! the near ones are cleared again before each ground click), then:
//!
//! 1. the door is still closed (`near(object, C).mode_id == 0`) and the
//!    player on its positive side (`ay >= 1`, or `ax`), so the clicks that
//!    follow cross it;
//! 2. once the player is out of hit recovery (a click while the player
//!    reels from a hit does nothing), a click on it opens it
//!    (`mode_id == 2`);
//! 3. a ground click across the narrow axis, [`CROSS`] sub-tiles from the
//!    player, beyond the door, brings the player through
//!    (`near(object, C).ay <= -2`, or `ax` for a door that runs along y);
//! 4. a ground click [`CROSS`] sub-tiles back brings the player back through
//!    (`ay >= 2`).
//!
//! Steps 2 to 4 wait for their result (`input wait-until`) before the next
//! click, so a late step moves the later ones with it; a result that never
//! comes fails the case at that step. A `by` check would not do: it reads
//! from tick 0, before the `goto`, when the player may stand anywhere.
//!
//! The ground clicks are measured from the player, not from the door: a
//! long door can stand over wall cells, so its position need not lie on
//! the way through (the Maggot Lair's slime door 230 is 7 long over a
//! 3-wide tunnel, two cells off its middle), and a click straight out from
//! it lands in a wall. They reach well past the door: a click on the open
//! door's drawn sprite, which stands tall above its cells, is a click on the
//! door, not on the ground behind it (tomb door 92); a click point in a
//! wall resolves to the nearest free cell on the way back, still beyond the
//! door.
//!
//! The doors come from the level door table the map builder places them
//! from; a door's narrow axis is the smaller side of its footprint
//! (`SizeX` / `SizeY`). Every object with `IsDoor` belongs to Acts I and
//! II: Acts III and IV have no doors, and the Act V rows of the table are
//! barricades (monsters, left out), the town gate and a portal (objects
//! without `IsDoor`, counted skips).
//!
//! A door spawns closed unless its data says otherwise: a class with
//! `PreOperate` (most Act I and II wooden and iron doors, 13-16, 62-64,
//! 290-295) spawns open on a 1-in-14 object-seed draw. That is a legitimate
//! state the case does not start from, hence the `mode 0` on the `goto`.
//!
//! A table row places a door only where the level's map uses its wall
//! tile, so a class may be missing from the seed's map (or every door of
//! it spawned open): its `goto` then searches every room and ends
//! `failed (none in level N)`, which the sweep counts as a
//! `target-not-on-map` skip, not a failure.
use super::*;
use d2_data::tables::{Leveldefs, Objects};
use d2_sim::drlg::preset::PresetTables;

/// Sub-tiles from the player each ground click across the door aims at.
const CROSS: i32 = 11;

/// The screen offset, in frame pixels, of a point `(dx, dy)` sub-tiles from
/// a unit's feet.
fn pixels(dx: i32, dy: i32) -> (i32, i32) {
    ((dx - dy) * 16, (dx + dy) * 8)
}

pub(super) fn generate(live: &LiveData, g: &mut Generated) -> Result<()> {
    let levels = live.tables.rows::<Leveldefs>()?;
    let objects = live.tables.rows::<Objects>()?;
    let tables = PresetTables::spec().map_err(|e| anyhow!("preset tables: {e:?}"))?;
    for (level, rows) in &tables.doors {
        let level = *level as usize;
        let Some(act) = live.waypoints.levels.get(level).map(|l| l.act) else {
            g.skip("doors:level-not-loaded");
            continue;
        };
        if !levels.get(level).is_some_and(|l| l.drlgtype > 0) || act >= 5 {
            g.skip("doors:level-not-playable");
            continue;
        }
        let mut classes: Vec<usize> = rows
            .iter()
            .filter(|r| r.unit_type == 2 && r.class >= 0)
            .map(|r| r.class as usize)
            .collect();
        classes.sort_unstable();
        classes.dedup();
        for class in classes {
            let Some(row) = objects.get(class).filter(|o| o.isdoor != 0) else {
                g.skip("doors:table-class-not-a-door");
                continue;
            };
            // Across the narrow side of the footprint, from the player (who
            // stands a few sub-tiles out from the opening on the positive
            // side): through the door, then back.
            let (back, on) = (-CROSS, CROSS);
            let (axis, near, far) = if row.sizex < row.sizey {
                ("ax", pixels(back, 0), pixels(on, 0))
            } else {
                ("ay", pixels(0, back), pixels(0, on))
            };
            let door = format!("near(object, {class})");
            let steps = format!(
                "at 0 dev god on\n\
                 at 1 poke goto preset {level} 2:{class} mode 0 opening\n\
                 at 424 dev kill level\n\
                 at 425 dev kill screen\n\
                 at 440 check {door}.mode_id == 0\n\
                 at 440 check {door}.{axis} >= 1\n\
                 at 441 input wait-until player.mode_id != 4 max 100\n\
                 at 442 input clickunit 2 {class} 0 -20\n\
                 at 443 input wait-until {door}.mode_id == 2 max 100\n\
                 at 552 dev kill screen\n\
                 at 560 input clickunit 0 0 {} {}\n\
                 at 561 input wait-until {door}.{axis} <= -2 max 150\n\
                 at 712 dev kill screen\n\
                 at 720 input clickunit 0 0 {} {}\n\
                 at 721 input wait-until {door}.{axis} >= 2 max 150",
                near.0, near.1, far.0, far.1
            );
            g.add(
                "doors",
                format!("level-{level}-door-{class}"),
                &format!("char class 0\nchar area {act} {level}"),
                &steps,
                1000,
            )?;
        }
    }
    Ok(())
}
