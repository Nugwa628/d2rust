//! Client-rate regression cases on synthetic data.
use super::world::{ClientUnit, ClientWorld, ModelInputs, MonsterSequence, UnitKey, PLAYER};
use super::{client_missiles as missiles, monster_anim, player_anim};
use d2_sim::tick::TickRate;

#[test]
fn player_monster_and_sequence_animations_cover_the_same_second() {
    for hz in [25, 60] {
        let rate = TickRate::new(hz).unwrap();
        let key = UnitKey::new(PLAYER, 1);
        let mut world = ClientWorld {
            tick_rate: rate,
            ..ClientWorld::default()
        };
        let mut player = ClientUnit::new(key);
        player.mode = 1;
        player.speed = Some(213);
        player.frame_count = 100 << 8;
        world.units.insert(key, player.clone());
        let mut monster = player.clone();
        let mut sequence = player;
        sequence.sequence = Some(MonsterSequence {
            frames: vec![
                monster_anim::SeqFrame {
                    mode: 1,
                    frame: 0,
                    event: 0
                };
                100
            ],
            length: 100 << 8,
            pos: 0,
            step: 179,
            drawn_mode: 1,
        });
        for frame in 0..hz as i32 {
            world.server_frame = Some(frame);
            player_anim::step(&mut world, &ModelInputs::default(), key).unwrap();
            monster_anim::advance_at(&mut monster, rate, frame);
            monster_anim::advance_at(&mut sequence, rate, frame);
        }
        assert_eq!(world.units[&key].frame, 213 * 25);
        assert_eq!(monster.frame, 213 * 25);
        assert_eq!(sequence.sequence.as_ref().unwrap().pos, 179 * 25);
        assert_eq!(sequence.frame_count, (100 << 8) - 179 * 25);
    }
}

#[test]
fn client_missile_lifetime_animation_and_distance_match_real_time() {
    let mut results = Vec::new();
    for hz in [25, 60] {
        let rate = TickRate::new(hz).unwrap();
        let mut world = ClientWorld {
            tick_rate: rate,
            ..ClientWorld::default()
        };
        let rows = [
            missiles::ClientMissileRow::default(),
            missiles::ClientMissileRow {
                vel: 8,
                range: 50,
                anim_len: 200,
                anim_speed: 16,
                clt_do_func: 1,
                ..missiles::ClientMissileRow::default()
            },
        ];
        let key = missiles::create(
            &mut world,
            &rows,
            &missiles::CreateRecord {
                flags: missiles::flag::POSITION | missiles::flag::TARGET_RELATIVE,
                class: 1,
                x: 100,
                y: 100,
                tx: 10000,
                ..missiles::CreateRecord::default()
            },
            false,
        )
        .unwrap()
        .unwrap();
        assert_eq!(world.objclient.missiles[&key].total, 2 * hz as i32);
        for frame in 0..hz as i32 {
            world.server_frame = Some(frame);
            missiles::update(&mut world, &rows, key, false).unwrap();
        }
        let missile = world.objclient.missiles[&key];
        assert_eq!(missile.current, hz as i32);
        assert_eq!(missile.frame, 25 << 8);
        results.push(missile.pos);
        for frame in hz as i32..2 * hz as i32 {
            world.server_frame = Some(frame);
            missiles::update(&mut world, &rows, key, false).unwrap();
        }
        assert!(!world.objclient.missiles.contains_key(&key));
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn day_clock_and_camera_keep_their_real_time_at_each_rate() {
    use crate::rules::{
        camera::Shake,
        lighting::environment::{Environment, PeriodTables},
    };
    let tables = PeriodTables::builtin().unwrap();
    for level in [1, 75, 103, 109] {
        let mut results = Vec::new();
        for hz in [25, 60] {
            let rate = TickRate::new(hz).unwrap();
            let mut env = Environment::new(&tables, 0);
            for frame in 0..hz as i32 {
                env.update_at(&tables, level, rate, frame);
            }
            results.push(env);
            assert_eq!(Shake::time_at(hz, rate), 1000);
        }
        assert_eq!(results[0], results[1], "level {level}");
    }
}

#[test]
fn authoritative_share_phase_is_independent_of_client_join_time() {
    let rate = TickRate::new(60).unwrap();
    let mut world = ClientWorld {
        tick_rate: rate,
        server_ticks: 1,
        server_frame: Some(1001),
        ..ClientWorld::default()
    };
    assert_eq!(rate.share(213, world.share_frame()), rate.share(213, 1001));
    world.server_frame = Some(i32::MIN);
    assert_eq!(world.share_frame(), i32::MIN);
    world.server_frame = None;
    assert_eq!(world.share_frame(), 1);
}

/// F5: attack (A1) and cast (SC) animations end after the same real time
/// at 25 Hz and at the game's 60 Hz: 20 frames at one a base tick end on
/// the update whose advanced frame reaches the count (760 ms at 25 Hz),
/// within one base tick at 60 Hz.
#[test]
fn attack_and_cast_animations_end_after_the_same_real_time() {
    for hz in [25, 60] {
        let rate = TickRate::new(hz).unwrap();
        for mode in [7, 10] {
            let key = UnitKey::new(PLAYER, 1);
            let mut world = ClientWorld {
                tick_rate: rate,
                ..ClientWorld::default()
            };
            let mut u = ClientUnit::new(key);
            u.mode = mode;
            u.speed = Some(256);
            u.frame_count = 20 << 8;
            world.units.insert(key, u);
            let mut t = 0;
            while world.units[&key].mode == mode {
                t += 1;
                world.server_frame = Some(t);
                player_anim::step(&mut world, &ModelInputs::default(), key).unwrap();
                assert!(t < 1000, "mode {mode} ends");
            }
            let ms = i64::from(t) * 1000 / i64::from(hz);
            assert!(
                (ms - 760).abs() <= 40,
                "mode {mode} at {hz} Hz: ended after {ms} ms"
            );
        }
    }
}
