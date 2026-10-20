//! Animation playback rate: every class's idle, walk, run, attack and cast,
//! monsters and town NPCs, drawn at the rate of their speed tables.
//!
//! Stated rule: a unit's animation plays `speed / 256` frames per base
//! tick of 25 Hz (the speed the server holds for the unit's mode), so a
//! walk, an attack or an idle loop lasts the same time in seconds whatever
//! the logic tick rate. Each case puts a unit in a mode and checks, through
//! `anim_rate(TYPE, CLASS, MODE).ratio`, that the frames the world view drew
//! per second are that rate within 15 %, over at least a third of a second
//! of the mode (`ANIM_MIN_TICKS`).
use super::*;
use d2_data::tables::{Leveldefs, Monstats, Monstats2, Skills};
use d2_sim::monsters::population::placement::water_only;
use d2_sim::world::vendors::TOWNS;

/// How far the drawn rate may be from the table rate: about a frame more
/// or less over the shortest measured stretch.
const TOLERANCE: &str = "0.15";

/// Walking as a player does: the left button held right of the player for
/// a little over two seconds.
const HOLD: &str = "at 2 input hold 560 300 140";

/// The player's cast mode: a skill whose `anim` is this plays the cast.
const CAST_MODE: u8 = 10;

/// `monai` row 140 (BaalCrabClone): a clone with no living owner is killed
/// on its first think, so one spawned on its own is never drawn.
const AI_BAAL_CLONE: u16 = 140;

/// Why a spawnable monster cannot be measured where the sweep spawns it,
/// read from its `monstats` / `monstats2` rows; `None` when it can.
fn unmeasurable(class: usize, m: &Monstats, m2: Option<&Monstats2>) -> Option<&'static str> {
    if m.ai == AI_BAAL_CLONE {
        return Some("anims:monster-owner-only");
    }
    let m2 = m2?;
    // A spawned monster stands in neutral: a class with no neutral art
    // (bird2, the fish critter) draws nothing.
    if !m2.mnu {
        return Some("anims:monster-no-neutral-art");
    }
    // Water-only placement (the frog demons): the Act I field has no water.
    if water_only(class as i32, m2.spawncol) {
        return Some("anims:monster-water-spawn");
    }
    None
}

pub(super) fn generate(live: &LiveData, g: &mut Generated) -> Result<()> {
    let levels = live.tables.rows::<Leveldefs>()?;
    let monsters = live.tables.rows::<Monstats>()?;
    let monsters2 = live.tables.rows::<Monstats2>()?;
    let skills = live.tables.rows::<Skills>()?;
    // The first Act I field (not a town) with a generator.
    let field = (1..levels.len()).find(|&id| {
        levels[id].drlgtype > 0
            && live.waypoints.levels.get(id).is_some_and(|l| l.act == 0)
            && !TOWNS.contains(&(id as u16))
    });
    let Some(field) = field else {
        g.skip("anims:no-act1-field");
        return Ok(());
    };
    let target = monsters
        .iter()
        .position(|m| m.enabled && !m.npc && m.killable);
    let check = |ty: &str, class: &str, mode: &str, by: u32| {
        format!("by {by} check anim_rate({ty}, {class}, {mode}).ratio ~ 1 {TOLERANCE}")
    };
    for class in 0..7u8 {
        let town = format!("char class {class}\nchar area 0 {}", TOWNS[0]);
        let out = format!("char class {class}\nchar area 0 {field}");
        // Movement goes through the client's own input, as a player's does:
        // the left button held right of the player (a message sent past the
        // client would leave the client drawing its player standing). The
        // field's neutral case takes one step first (a single click): the
        // warp out of town leaves the player in town neutral until it moves.
        let god = "at 0 dev god on";
        for (name, header, steps, mode) in [
            ("town-neutral", &town, String::new(), "town_neutral"),
            ("town-walk", &town, HOLD.to_string(), "town_walk"),
            (
                "neutral",
                &out,
                format!("{god}\nat 2 input click 560 300"),
                "neutral",
            ),
            ("walk", &out, format!("{god}\n{HOLD}"), "walk"),
            (
                "run",
                &out,
                format!("{god}\nat 1 input action toggle_run\n{HOLD}"),
                "run",
            ),
        ] {
            // Standing after a walk starts once the client's player stops,
            // a while after the button is let go.
            let by = if name == "neutral" { 300 } else { 150 };
            let steps = format!("{steps}\n{}", check("player", "*", mode, by));
            g.add(
                "anims",
                format!("class-{class}-{name}"),
                header,
                steps.trim_start(),
                by + 4,
            )?;
        }
        let Some(target) = target else {
            g.skip("anims:no-attack-target");
            continue;
        };
        // A standing target next to the player; the left button attacks.
        g.add("anims", format!("class-{class}-attack1"), &out,
            &format!("at 0 dev god on\nat 1 spawn {target} @x+2 @y normal\nat 8 input clickunit 1 {target}\n{}", check("player", "*", "attack1", 120)), 124)?;
        // The class's first cast-mode skill a level 1 character can learn,
        // aimed at the same target.
        let cast = skills.iter().find(|s| {
            s.charclass == class
                && s.anim == CAST_MODE
                && s.reqlevel <= 1
                && !s.passive
                && !s.aura
                && !(s.targetcorpse || s.targetitem || s.targetpet || s.targetally)
        });
        let Some(cast) = cast else {
            g.skip("anims:no-level-1-cast-skill");
            continue;
        };
        let header = format!(
            "{out}\nchar skill {} 1\nchar hand right {}\nchar stat 8 256000\nchar stat 9 256000",
            cast.skill, cast.skill
        );
        g.add("anims", format!("class-{class}-cast-{}", cast.skill), &header,
            &format!("at 0 dev god on\nat 1 spawn {target} @x+6 @y normal\nat 8 input cast {} at unit 1 {target}\n{}", cast.skill, check("player", "*", "cast", 120)), 124)?;
    }
    // Monsters as they stand, walk or fight near the player: whatever mode
    // they are drawn in, once it lasted a third of a second.
    for (class, m) in monsters.iter().enumerate() {
        if !m.enabled || m.npc || !m.killable {
            g.skip("anims:monster-not-spawnable");
            continue;
        }
        if let Some(why) = unmeasurable(class, m, monsters2.get(usize::from(m.monstatsex))) {
            g.skip(why);
            continue;
        }
        g.add(
            "anims",
            format!("monster-{class}"),
            &format!("char class 0\nchar area 0 {field}"),
            &format!(
                "at 0 dev god on\nat 1 spawn {class} @x+10 @y normal\n{}",
                check("monster", &class.to_string(), "*", 300)
            ),
            304,
        )?;
    }
    // Town NPCs in every act: the first one in view.
    for (act, town) in TOWNS.iter().enumerate() {
        if !levels
            .get(usize::from(*town))
            .is_some_and(|l| l.drlgtype > 0)
        {
            g.skip("anims:town-not-playable");
            continue;
        }
        g.add(
            "anims",
            format!("act-{act}-town-npc"),
            &format!("char class 0\nchar area {act} {town}"),
            &check("monster", "*", "*", 300),
            304,
        )?;
    }
    Ok(())
}
