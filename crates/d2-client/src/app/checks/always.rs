//! Rules that run alongside the shared soak invariants. All inputs are
//! borrowed snapshots; bookkeeping lives outside the game.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::{v, Desync, GuidHistory, ServerView, Violation};
use crate::app::single_player::{self, Sim};
use crate::bridge::world::{ClientWorld, UnitKey, MONSTER, PLAYER};
use d2_sim::stats::stat;
use d2_sim::units::UnitType;

/// Additional server fields, sampled between ticks.
#[derive(Default)]
pub struct View {
    pub hud: BTreeMap<&'static str, i32>,
    pub blocked: Vec<(u8, u32, i32, i32, u16)>,
}

pub fn server_view(sim: &Sim) -> View {
    let mut view = View::default();
    let sys = &sim.events.action.sys;
    if let Some((id, _)) = single_player::local_player(sim) {
        for (name, stat, shift) in [
            ("life", stat::HITPOINTS, 8),
            ("life_max", stat::MAXHP, 8),
            ("mana", stat::MANA, 8),
            ("mana_max", stat::MAXMANA, 8),
            ("stamina", stat::STAMINA, 8),
            ("stamina_max", stat::MAXSTAMINA, 8),
            ("level", stat::LEVEL, 0),
            ("exp", 13, 0),
            ("gold", 14, 0),
        ] {
            // Vitals/experience/gold have a dedicated sync. Other stats
            // reach the HUD through the saved-stat mod array; synthetic
            // installs may deliberately leave that capability disabled.
            if !matches!(stat, 6 | 8 | 10 | 13 | 14)
                && !sys
                    .stats
                    .data()
                    .stats
                    .get(stat)
                    .is_some_and(|row| row.saved)
            {
                continue;
            }
            view.hud
                .insert(name, sys.stats.unit_total(id, stat, 0) >> shift);
        }
    }
    // Test the terrain bits only: an actor stamps its own dynamic footprint.
    // Respect each actor's movement mask, including flying monsters.
    if let Some(paths) = &sys.hooks.paths {
        for ty in [UnitType::Player, UnitType::Monster] {
            for id in sim.game.lists.units_of_type(ty) {
                let Some(unit) = sys.units.get(id) else {
                    continue;
                };
                if unit.is_dead() {
                    continue;
                }
                // A scripted sequence (player 18, monster 14) is not a walk:
                // the unit may sit on a wall inside its spawn animation, as
                // the preset monsters in the act II tombs do. A monster that
                // never moves (a barricade door or tower) or whose table row
                // places it off the ground (inert props, water-spawned
                // monsters) stands on the cells its preset blocks: the rule
                // is about the cells a mover stands on.
                let off_land = ty == UnitType::Monster
                    && (sys
                        .hooks
                        .tables
                        .combat
                        .monstats2(unit.class as i32)
                        .is_some_and(|m| m.inert || matches!(m.spawncol, 1 | 3))
                        || sys
                            .hooks
                            .tables
                            .combat
                            .monstats
                            .get(unit.class as usize)
                            .is_some_and(|m| never_moves(m.velocity, m.run)));
                if !terrain_applies(ty, unit.mode, off_land) {
                    continue;
                }
                let Some(path) = paths.record(id) else {
                    continue;
                };
                let terrain = paths.dynamic(id).map_or(0, |p| p.move_mask & 0x0009);
                if terrain == 0 {
                    continue;
                }
                let (x, y) = path.position();
                // NO_PATH also marks the actor's own centre, so omit it.
                let mask = d2_sim::path::collision::point_value(
                    &sys.hooks.drlg,
                    path.room(),
                    x,
                    y,
                    terrain,
                );
                if mask != 0 {
                    view.blocked.push((ty as u8, unit.guid, x, y, mask));
                }
            }
        }
    }
    view
}

/// The scripted-sequence modes (player 18, monster 14): their frames advance
/// through a sequence, not a loop, and their position is the script's.
fn is_sequence_mode(ty: UnitType, mode: u32) -> bool {
    matches!((ty, mode), (UnitType::Player, 18) | (UnitType::Monster, 14))
}

/// Whether the terrain under an actor must be walkable for it: a scripted
/// sequence is not a walk, and a monster whose table row places it off the
/// ground (`off_land`: an inert prop such as the act V barricade doors, a
/// water-spawned monster with spawn collision 1, or spawn collision 3 for any
/// cell) stands on a blocked cell by design.
fn terrain_applies(ty: UnitType, mode: u32, off_land: bool) -> bool {
    !is_sequence_mode(ty, mode) && !(ty == UnitType::Monster && off_land)
}

/// A monster row with no walk and no run speed stays where it was placed.
fn never_moves(velocity: u16, run: u16) -> bool {
    velocity == 0 && run == 0
}

/// State 143 `attached`: the unit rides the unit it is linked to.
const STATE_ATTACHED: u8 = 143;
/// The monster mode an attached rider holds while it rides (SC).
const RIDING_MODE: u32 = 7;

/// Limits in server ticks. Movement may last indefinitely while making
/// progress; consecutive casts are distinguished by their mode requests.
pub fn mode_limit(ty: u8, mode: u32) -> Option<u32> {
    match (ty, mode) {
        (PLAYER, 0 | 1 | 5 | 17) | (MONSTER, 0 | 1 | 12) => None,
        (PLAYER, 2 | 3 | 6) | (MONSTER, 2 | 15) => Some(250),
        (PLAYER, 4 | 7..=16 | 18 | 19) | (MONSTER, 3..=11 | 13 | 14) => Some(125),
        _ => None,
    }
}

#[derive(Default)]
pub struct Always {
    guids: GuidHistory,
    desync: Desync,
    modes: BTreeMap<UnitKey, Progress>,
    hud_streak: BTreeMap<String, u32>,
    seen: BTreeSet<String>,
}

struct Progress {
    mode: u32,
    ticks: u32,
    position: Option<(u16, u16)>,
    requests: u32,
}

impl Always {
    /// Reports a repeated mismatch after its publication deadline. The
    /// vitals sync may wait 20 updates before forcing a flush; allow two
    /// client passes after that. Saved-stat writes need only those passes.
    pub fn observe(
        &mut self,
        world: &ClientWorld,
        server: &ServerView,
        extra: &View,
        state: &Value,
    ) -> Vec<Violation> {
        let mut out = self.guids.observe(&server.units);
        out.extend(self.desync.observe(world, server));
        let mut mismatches = BTreeSet::new();
        for (&name, &expected) in &extra.hud {
            let actual = state["hud"][name].as_i64();
            if actual != Some(i64::from(expected)) && !server.player_dead {
                let sig = format!("hud:{}:{name}", server.player_guid.unwrap_or(0));
                mismatches.insert(sig.clone());
                let count = self.hud_streak.entry(sig.clone()).or_default();
                *count += 1;
                let grace = if matches!(name, "life" | "mana" | "stamina" | "gold" | "exp") {
                    22
                } else {
                    3
                };
                if *count == grace {
                    out.push(v(
                        "hud",
                        sig,
                        format!("HUD {name} {actual:?}, server {expected}"),
                    ));
                }
            }
        }
        self.hud_streak.retain(|sig, _| mismatches.contains(sig));
        self.modes.retain(|key, _| world.units.contains_key(key));
        for (key, unit) in &world.units {
            // A sequence animation (player mode 18, monster mode 14) sets the
            // frame from the current sequence frame's own animation and counts
            // down the remaining sequence length: the two numbers are not a
            // loop's position and length, so the bound does not apply. An
            // object's frame is a COF offset (a barrel's operate animation
            // starts at 2304 while its mode has 256 frames), so the bound
            // only applies to players and monsters.
            if matches!(key.unit_type, PLAYER | MONSTER)
                && unit.sequence.is_none()
                && unit.frame_count > 0
                && (unit.frame < 0 || unit.frame >= unit.frame_count)
            {
                out.push(v(
                    "animation",
                    format!("animation:{}:{}", key.unit_type, key.guid),
                    format!("frame {} outside [0, {})", unit.frame, unit.frame_count),
                ));
            }
            // A scripted sequence (player 18, monster 14) is not a walk or a
            // held mode: its progress is the sequence position, and the unit's
            // own position and mode requests stay fixed for its length. The
            // sequence is not bounded by the walk/cast limits.
            // A rider (state `attached`: an imp on a siege beast or tower)
            // holds its riding mode (monster mode 7, SC) and follows its
            // mount for as long as it rides; the ride ends by a server mode
            // change, not by the mode. Its other modes keep the limits.
            let riding = key.unit_type == MONSTER
                && unit.mode == RIDING_MODE
                && unit.states.contains(&STATE_ATTACHED);
            if matches!((key.unit_type, unit.mode), (PLAYER, 18) | (MONSTER, 14)) || riding {
                self.modes.remove(key);
                continue;
            }
            let Some(limit) = mode_limit(key.unit_type, unit.mode) else {
                self.modes.remove(key);
                continue;
            };
            let progress = || Progress {
                mode: unit.mode,
                ticks: 0,
                position: unit.position,
                requests: unit.mode_requests,
            };
            let entry = self.modes.entry(*key).or_insert_with(progress);
            if entry.mode != unit.mode
                || entry.position != unit.position
                || entry.requests != unit.mode_requests
            {
                *entry = progress();
            }
            entry.ticks += 1;
            if entry.ticks > limit {
                out.push(v(
                    "stuck",
                    format!("stuck:{}:{}:{}", key.unit_type, key.guid, unit.mode),
                    format!(
                        "mode {} made no progress for {} ticks (limit {limit})",
                        unit.mode, entry.ticks
                    ),
                ));
            }
        }
        for &(ty, guid, x, y, mask) in &extra.blocked {
            out.push(v(
                "walkable",
                format!("walkable:{ty}:{guid}"),
                format!("unit {ty}:{guid} at ({x}, {y}) has terrain mask {mask:#x}"),
            ));
        }
        out
    }

    /// Opt-outs precede deduplication, and signatures never contain a tick.
    pub fn retain_new(
        &mut self,
        violations: Vec<Violation>,
        allow: &BTreeSet<String>,
    ) -> Vec<Violation> {
        violations
            .into_iter()
            .filter(|v| !allow.contains(v.kind) && self.seen.insert(v.signature()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::{ClientUnit, OBJECT};
    use serde_json::json;

    #[test]
    fn hud_mismatch_must_survive_the_pipeline() {
        let mut checks = Always::default();
        let server = ServerView {
            player_guid: Some(7),
            ..Default::default()
        };
        let extra = View {
            hud: [("gold", 100)].into(),
            ..Default::default()
        };
        let world = ClientWorld::default();
        let good = json!({"hud":{"gold":100}});
        assert!(checks.observe(&world, &server, &extra, &good).is_empty());
        let bad = json!({"hud":{"gold":99}});
        for _ in 0..21 {
            assert!(checks.observe(&world, &server, &extra, &bad).is_empty());
        }
        assert_eq!(checks.observe(&world, &server, &extra, &bad)[0].kind, "hud");
        assert!(checks.observe(&world, &server, &extra, &good).is_empty());
        let saved = View {
            hud: [("level", 1)].into(),
            ..Default::default()
        };
        let bad = json!({"hud":{"level":0}});
        for _ in 0..2 {
            assert!(checks.observe(&world, &server, &saved, &bad).is_empty());
        }
        assert_eq!(checks.observe(&world, &server, &saved, &bad)[0].kind, "hud");
    }

    #[test]
    fn a_door_or_tower_never_moves_but_a_walker_does() {
        // barricadedoor2 / barricadetower rows: Velocity 0, Run 0.
        assert!(never_moves(0, 0));
        assert!(!never_moves(4, 4));
        assert!(!never_moves(0, 6));
    }

    #[test]
    fn blocked_position_reports_the_unit() {
        let mut checks = Always::default();
        let good = View::default();
        assert!(checks
            .observe(
                &ClientWorld::default(),
                &ServerView::default(),
                &good,
                &Value::Null
            )
            .is_empty());
        let bad = View {
            blocked: vec![(PLAYER, 7, 10, 20, 1)],
            ..good
        };
        assert_eq!(
            checks.observe(
                &ClientWorld::default(),
                &ServerView::default(),
                &bad,
                &Value::Null
            )[0]
            .sig,
            "walkable:0:7"
        );
    }

    fn actor() -> ClientWorld {
        let mut world = ClientWorld::default();
        let key = UnitKey::new(PLAYER, 7);
        let mut unit = ClientUnit::new(key);
        unit.mode = 10;
        unit.frame_count = 2560;
        world.units.insert(key, unit);
        world
    }

    #[test]
    fn animation_frame_has_a_strict_upper_bound() {
        let mut checks = Always::default();
        let mut world = actor();
        assert!(checks
            .observe(
                &world,
                &ServerView::default(),
                &View::default(),
                &Value::Null
            )
            .is_empty());
        world.units.get_mut(&UnitKey::new(PLAYER, 7)).unwrap().frame = 2560;
        assert_eq!(
            checks.observe(
                &world,
                &ServerView::default(),
                &View::default(),
                &Value::Null
            )[0]
            .kind,
            "animation"
        );
    }

    #[test]
    fn an_object_frame_is_a_cof_offset_not_a_loop_index() {
        let mut checks = Always::default();
        let mut world = ClientWorld::default();
        let key = UnitKey::new(OBJECT, 26);
        let mut unit = ClientUnit::new(key);
        unit.mode = 2;
        unit.frame = 2304;
        unit.frame_count = 256;
        world.units.insert(key, unit);
        assert!(checks
            .observe(
                &world,
                &ServerView::default(),
                &View::default(),
                &Value::Null
            )
            .is_empty());
    }

    #[test]
    fn sequence_modes_are_not_stuck() {
        for (ty, mode) in [(PLAYER, 18), (MONSTER, 14)] {
            let mut checks = Always::default();
            let mut world = ClientWorld::default();
            let key = UnitKey::new(ty, 7);
            let mut unit = ClientUnit::new(key);
            unit.mode = mode;
            world.units.insert(key, unit);
            for _ in 0..300 {
                assert!(checks
                    .observe(
                        &world,
                        &ServerView::default(),
                        &View::default(),
                        &Value::Null
                    )
                    .is_empty());
            }
        }
    }

    /// A siege beast's imp rider holds mode 7 (SC) at one point while its
    /// mount stands; the held mode is the ride, not a stuck cast. The same
    /// imp without the state is stuck past the cast limit, and so is an
    /// attached imp held in another mode (get hit, 3).
    #[test]
    fn an_attached_rider_is_not_stuck() {
        for (attached, mode, stuck) in [(true, 7, false), (false, 7, true), (true, 3, true)] {
            let mut checks = Always::default();
            let mut world = ClientWorld::default();
            let key = UnitKey::new(MONSTER, 40);
            let mut unit = ClientUnit::new(key);
            unit.mode = mode;
            unit.position = Some((5712, 4943));
            if attached {
                unit.states.insert(STATE_ATTACHED);
            }
            world.units.insert(key, unit);
            let found = (0..200).any(|_| {
                checks
                    .observe(
                        &world,
                        &ServerView::default(),
                        &View::default(),
                        &Value::Null,
                    )
                    .iter()
                    .any(|v| v.kind == "stuck")
            });
            assert_eq!(found, stuck, "attached {attached}, mode {mode}");
        }
    }

    #[test]
    fn terrain_rules_skip_sequence_and_static_props() {
        assert!(!terrain_applies(UnitType::Monster, 14, false));
        assert!(!terrain_applies(UnitType::Player, 18, false));
        assert!(!terrain_applies(UnitType::Monster, 1, true));
        assert!(terrain_applies(UnitType::Monster, 1, false));
        assert!(terrain_applies(UnitType::Player, 2, false));
    }

    #[test]
    fn a_sequence_frame_is_not_bounded_by_the_remaining_sequence_count() {
        let mut checks = Always::default();
        let mut world = actor();
        let key = UnitKey::new(PLAYER, 7);
        let u = world.units.get_mut(&key).unwrap();
        u.sequence = Some(crate::bridge::world::MonsterSequence {
            frames: Vec::new(),
            length: 0x1000,
            pos: 0,
            step: 0x100,
            drawn_mode: 18,
        });
        u.frame = 2048;
        u.frame_count = 1707;
        assert!(checks
            .observe(
                &world,
                &ServerView::default(),
                &View::default(),
                &Value::Null
            )
            .is_empty());
    }

    #[test]
    fn stuck_modes_reset_on_progress_and_exempt_idle_and_death() {
        let mut checks = Always::default();
        let mut world = actor();
        for _ in 0..125 {
            assert!(checks
                .observe(
                    &world,
                    &ServerView::default(),
                    &View::default(),
                    &Value::Null
                )
                .is_empty());
        }
        assert_eq!(
            checks.observe(
                &world,
                &ServerView::default(),
                &View::default(),
                &Value::Null
            )[0]
            .kind,
            "stuck"
        );
        world
            .units
            .get_mut(&UnitKey::new(PLAYER, 7))
            .unwrap()
            .mode_requests += 1;
        assert!(checks
            .observe(
                &world,
                &ServerView::default(),
                &View::default(),
                &Value::Null
            )
            .is_empty());
        for mode in [0, 1, 5, 17] {
            assert_eq!(mode_limit(PLAYER, mode), None);
        }
        for mode in [0, 1, 12] {
            assert_eq!(mode_limit(MONSTER, mode), None);
        }
    }

    #[test]
    fn opt_out_and_signature_deduplication() {
        let mut checks = Always::default();
        let broken = || vec![v("hud", "hud:7:gold".into(), "bad gold".into())];
        assert!(checks
            .retain_new(broken(), &["hud".into()].into())
            .is_empty());
        assert_eq!(checks.retain_new(broken(), &BTreeSet::new()).len(), 1);
        assert!(checks.retain_new(broken(), &BTreeSet::new()).is_empty());
    }
}
