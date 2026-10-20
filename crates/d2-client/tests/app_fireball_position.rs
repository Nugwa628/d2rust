//! Fire Ball's complete input path on the install-backed local server,
//! without a window. Sample server, prediction, view-feed and smoothing
//! positions at display rate, including held repeats and release.

use crate::real_rig::Rig;
use bevy::prelude::*;
use d2_client::bridge::click::{screen_to_world, ClickView};
use d2_client::bridge::{BridgeResource, UnitKey};
use d2_client::controls::click::{mods, ClickState, Kind};
use d2_client::rules::camera::{moving_to_client, Camera, FrameSize, OpenMode, UnitPosition};
use d2_client::world_view::unit_motion::{Seen, UnitMotion};
use d2_client::world_view::walk::PreviewWalk;
use d2_client::world_view::WorldViewState;
use std::sync::atomic::Ordering;

fn server_sample(r: &mut Rig) -> ((u32, u32), u32, Option<i32>, usize) {
    r.with(|sim, p| {
        let path = sim
            .events
            .action
            .hooks()
            .paths
            .as_ref()
            .unwrap()
            .dynamic(p)
            .unwrap();
        (
            (path.precise_x, path.precise_y),
            sim.events.action.sys.units.get(p).unwrap().mode,
            sim.events
                .action
                .sys
                .hooks
                .used_skill_of(p)
                .map(|e| e.skill),
            sim.game
                .lists
                .units_of_type(d2_sim::units::UnitType::Missile)
                .len(),
        )
    })
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn fire_ball_input_matrix_stays_in_place_at_display_rate() {
    run_matrix(false, false);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn fire_ball_repeat_while_other_button_held_stays_in_place() {
    run_matrix(true, false);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn fire_ball_input_stays_in_place_with_one_tick_per_slow_frame() {
    run_matrix(false, true);
}

fn run_matrix(repeat_only: bool, legacy_pacing: bool) {
    let skill = crate::real_rig::skill_named("Fire Ball");
    let mut failures = Vec::new();
    let mut r = Rig::new("sorceress", &[skill]);
    r.leave_town();
    let home = r.pos();
    let mut cases = Vec::new();
    let profiles: &[u8] = if legacy_pacing {
        &[5]
    } else {
        &[4, 0, 1, 2, 3]
    };
    for &profile in profiles {
        for left in [false, true] {
            for distance in [8, 15] {
                for (unit, delayed_hover) in [(true, false), (true, true), (false, false)] {
                    if profile != 0 && delayed_hover {
                        continue;
                    }
                    for shift in [false, true] {
                        for held in [false, true] {
                            cases.push((profile, left, distance, unit, delayed_hover, shift, held));
                        }
                    }
                }
            }
        }
    }
    if repeat_only {
        cases.retain(|&(profile, left, _, _, delayed, shift, held)| {
            profile == 4 && !left && !delayed && !shift && held
        });
    }
    for (profile, left, distance, unit, delayed_hover, shift, held) in cases {
        r.leg(home);
        if legacy_pacing {
            // A one-tick capture policy also exercises casts when
            // simulation progress is limited by slow display frames.
            r.app
                .insert_resource(d2_client::bridge::mirror::DrawnTick(u64::MAX));
        }
        let walking = profile >= 2;
        let moving_target = profile != 0;
        let case = format!("profile={profile},walking={walking},moving_target={moving_target},left={left},distance={distance},unit={unit},delayed_hover={delayed_hover},shift={shift},held={held}");
        r.strengthen();
        let monster = r.spawn_monster(distance);
        let (player_guid, monster_guid) = r.with(move |sim, p| {
            sim.events.action.with(&mut sim.game, |_, v| {
                v.set_base(monster, d2_sim::stats::stat::HITPOINTS, 1_000 << 8);
                v.set_base(monster, d2_sim::stats::stat::MAXHP, 1_000 << 8);
                // Keep the target alive through repeats; death-to-ground
                // targeting is a separate input control.
                v.set_base(monster, 39, 100); // fire resistance
            });
            (
                sim.game.lists.unit(p).unwrap().guid,
                sim.game.lists.unit(monster).unwrap().guid,
            )
        });
        // The rig stages learned skills on the server. Give the
        // client the corresponding learned-skill message too.
        let mut learned = vec![0x94, 1];
        learned.extend(player_guid.to_le_bytes());
        learned.extend((skill as u16).to_le_bytes());
        learned.push(1);
        r.app
            .world_mut()
            .resource_mut::<BridgeResource>()
            .0
            .receive_chunk(&learned)
            .unwrap();
        // Select through the normal picker packet and its server
        // reply, rather than assigning either hand in the rig.
        r.send(&d2_client::ui::original::hud_hotkeys::select(skill as i32, left, u32::MAX).0);
        if profile == 4 {
            r.send(&d2_client::ui::original::hud_hotkeys::select(0, !left, u32::MAX).0);
        }
        r.step(4);
        {
            let bridge = &r.app.world().resource::<BridgeResource>().0;
            let list = bridge.world().local().unwrap().skills.as_ref().unwrap();
            let hand = if left { list.left } else { list.right }.unwrap();
            assert_eq!(usize::from(list.entries[hand].skill), skill);
        }
        r.app
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f64(1.0 / 60.0),
            ));
        if walking {
            let (x, y) = r.pos();
            let mut packet = vec![0x01];
            packet.extend(((x + 20) as u16).to_le_bytes());
            packet.extend((y as u16).to_le_bytes());
            r.send(&packet);
            r.step(2);
        }
        if moving_target {
            r.with(move |sim, _| {
                use d2_sim::monsters::ai::{AiModes, ModeTarget};
                let (x, y) = sim.events.action.sys.hooks.path_position(monster);
                sim.events.action.with(&mut sim.game, |g, v| {
                    assert!(v.change_mode(g, monster, 2, ModeTarget::Point(x, y + 15)));
                });
            });
        }
        let before = server_sample(&mut r).0;
        let tick_moved = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen_move = tick_moved.clone();
        let tick_case = case.clone();
        let player_id = r.with(|_, p| p);
        r.link.lock().unwrap().with(move |l| {
            l.set_tick_end(Box::new(move |h| {
                let sim = &mut h.game;
                let target_life = sim.events.action.with(&mut sim.game, |_, v| v.stat(monster, d2_sim::stats::stat::HITPOINTS));
                let paths = sim.events.action.sys.hooks.paths.as_ref().unwrap();
                let p = paths.dynamic(player_id).unwrap();
                let pos = (p.precise_x, p.precise_y);
                seen_move.fetch_or(pos != before, Ordering::SeqCst);
                let target_pos = paths.dynamic(monster).map(|p| (p.precise_x, p.precise_y));
                let target_mode = sim.events.action.sys.units.get(monster).unwrap().mode;
                println!("FIREBALL_TICK|{tick_case}|tick={}|server={pos:?}|server_mode={}|target={target_pos:?}|target_mode={target_mode}|target_life={target_life}", sim.game.frame, sim.events.action.sys.units.get(player_id).unwrap().mode);
            }));
        }).unwrap();
        println!("FIREBALL_CASE|{case}|before={before:?}");
        r.tx.lock().unwrap().clear();
        let target = UnitKey::new(1, monster_guid);
        let size = FrameSize::D2RUST;
        let camera = Camera::new(
            size,
            OpenMode::NONE,
            moving_to_client(before.0, before.1),
            (0, 0),
        );
        let goal = ((before.0 >> 16) as i32 + distance, (before.1 >> 16) as i32);
        // Project the centre of the target subtile, then check
        // the same inverse used by the click sender.
        let mouse = (
            16 * (goal.0 - goal.1) - camera.unit.x + camera.view.shift_x,
            8 * (goal.0 + goal.1) + 8 - camera.unit.y,
        );
        assert_eq!(screen_to_world(&camera, mouse.0, mouse.1), goal);
        let mut state = ClickState::default();
        // Profile 4 begins with the other button still held from a walk.
        if profile == 4 {
            state.left_held = !left;
            state.right_held = left;
            state.press = 1;
        }
        let mut motion = UnitMotion::default();
        let mut cast_frames = 0;
        let mut server_cast_frames = 0;
        let mut most_missiles = 0;
        let mut changed = false;
        let expected_cast = !(left && !shift && (!unit || delayed_hover));
        let release_frame = if held {
            if profile == 0 || legacy_pacing {
                55
            } else {
                18
            }
        } else {
            1
        };
        let frames = if profile == 0 || legacy_pacing {
            80
        } else {
            25
        };
        for frame in 0..frames {
            let (at, mode, speeds) = {
                let walk = r.app.world().resource::<PreviewWalk>();
                (walk.pump_position(), walk.predict.mode(), walk.speeds)
            };
            let view = ClickView {
                size,
                open_mode: 0,
                right_panel_bottom: size.play_height(),
                skill_y_limit: size.play_height(),
                mouse,
                game_menu_open: false,
                pick: false,
                shake: (0, 0),
                prev_hover: Some((unit && (!delayed_hover || frame > 0)).then_some(target)),
                local_mode: mode,
                speeds,
            };
            let flags = if shift { mods::STAND_STILL } else { 0 };
            {
                let mut bridge = r.app.world_mut().resource_mut::<BridgeResource>();
                if frame == 0 {
                    bridge
                        .0
                        .world_click_at(
                            &mut state,
                            view,
                            if left {
                                Kind::LeftDown
                            } else {
                                Kind::RightDown
                            },
                            Some(mouse),
                            flags,
                            at,
                        )
                        .unwrap();
                } else if frame == release_frame {
                    bridge
                        .0
                        .world_click_at(
                            &mut state,
                            view,
                            if left { Kind::LeftUp } else { Kind::RightUp },
                            Some(mouse),
                            flags,
                            at,
                        )
                        .unwrap();
                } else {
                    bridge
                        .0
                        .click_repeat_at(&mut state, view, flags, at)
                        .unwrap();
                }
            }
            if profile == 4 && frame == release_frame {
                state.input_reset();
            }
            state.end_pass();
            let frame_ms = match profile {
                0 => 17,
                1 => 100,
                2 => 50,
                5 => 100,
                _ => [50, 100, 67, 150, 50, 83][frame % 6],
            };
            r.app
                .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                    std::time::Duration::from_millis(frame_ms),
                ));
            r.ms.fetch_add(frame_ms as u32, Ordering::SeqCst);
            r.app.update();
            let (server, server_mode, used_skill, missiles) = server_sample(&mut r);
            let bridge = &r.app.world().resource::<BridgeResource>().0;
            let world = bridge.world();
            let player = world.local().unwrap();
            let target_state = world
                .units
                .get(&target)
                .map(|u| (u.mode, u.flag_4, u.position));
            let feed = &r.app.world().resource::<WorldViewState>().feed;
            let draw = match feed.unit_position(player).unwrap() {
                UnitPosition::Moving { x16, y16 } => (x16, y16),
                UnitPosition::Static { sx, sy } => ((sx as u32) << 16, (sy as u32) << 16),
            };
            motion.observe(
                world.server_ticks,
                [Seen {
                    key: player.key,
                    pos: draw,
                    placements: player.placements,
                    on_map: true,
                }],
            );
            let shifts = [0.0, 0.5, 1.0].map(|alpha| motion.shift(player.key, alpha));
            let offset = r
                .app
                .world()
                .resource::<d2_client::world_view::skill_motion::SkillMotion>()
                .offsets()
                .get(&player.key)
                .copied()
                .unwrap_or_default();
            let point = moving_to_client(draw.0, draw.1);
            let drawn_pixels = shifts.map(|s| (point.x + offset.0 + s.x, point.y + offset.1 + s.y));
            let prediction = r.app.world().resource::<PreviewWalk>().predict.position();
            let sent = std::mem::take(&mut *r.tx.lock().unwrap());
            println!("FIREBALL_TRACE|{case}|frame={frame}|tick={}|server={server:?}|server_mode={server_mode}|used_skill={used_skill:?}|missiles={missiles}|target_state={target_state:?}|model={:?}|prediction={prediction:?}|draw={draw:?}|pixels={drawn_pixels:?}|offset={offset:?}|smooth={shifts:?}|mode={}|held=({}, {})|sent={sent:02x?}", world.server_ticks, player.precise, player.mode, state.left_held, state.right_held);
            cast_frames += usize::from(player.mode == 10);
            server_cast_frames +=
                usize::from(server_mode == 10 && used_skill == Some(skill as i32));
            most_missiles = most_missiles.max(missiles);
            changed |= server != before
                || draw != before
                || offset != (0, 0)
                || shifts.iter().any(|s| *s != IVec2::ZERO);
        }
        if expected_cast
            && (changed
                || tick_moved.load(Ordering::SeqCst)
                || cast_frames == 0
                || server_cast_frames == 0
                || most_missiles == 0)
        {
            failures.push(format!(
                                "{case}: moved={changed}, cast_frames={cast_frames}, server_cast_frames={server_cast_frames}, most_missiles={most_missiles}"
                            ));
        }
        // Remove case fixtures before the next case; old targets and
        // missiles must not block a new target's movement or count as
        // evidence of a new cast.
        r.link
            .lock()
            .unwrap()
            .with(|l| l.set_tick_end(Box::new(|_| {})))
            .unwrap();
        r.with(move |sim, _| {
            let missiles = sim
                .game
                .lists
                .units_of_type(d2_sim::units::UnitType::Missile);
            sim.events.action.with(&mut sim.game, |g, v| {
                for missile in missiles {
                    v.remove(g, missile);
                }
                v.remove(g, monster);
            });
        });
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
