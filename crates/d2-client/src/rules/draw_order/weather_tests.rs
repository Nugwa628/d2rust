//! Tests of the weather rules and worked vectors.

use super::*;

const F800: FrameSize = FrameSize::D2RUST;

fn colors() -> ColorTables {
    ColorTables {
        period0: [10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21],
        period13: [30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41],
        period2: [50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61],
        snow: [70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81],
        snow_alt: [90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101],
    }
}

fn resources() -> ActResources {
    ActResources {
        splash_frames: [6, 6, 5, 4],
        bubble_frames: [8, 8, 7, 6],
        colors: colors(),
    }
}

fn level(level_id: u32, act: u8, rain: bool, mud: bool) -> LevelWeather {
    LevelWeather {
        level_id,
        act,
        rain,
        mud,
    }
}

fn input(c: u32) -> UpdateInput {
    UpdateInput {
        update_count: c,
        frame: F800,
        camera_delta: (0, 0),
        day_period: 2,
        video_mode: 3,
    }
}

fn mode(m: u8) -> OpenMode {
    OpenMode::new(m).unwrap()
}

/// A weather after act load in `lv` (act load draws on a scratch seed).
fn loaded(lv: LevelWeather) -> Weather {
    let mut w = Weather::new();
    let mut s = Seed::new(7, 0);
    w.act_load(
        LocalPlayer {
            seed: &mut s,
            level: lv,
        },
        resources(),
    )
    .unwrap();
    w
}

fn rec(x: i32, y: i32, kind: u8) -> PoolRecord {
    PoolRecord {
        x,
        y,
        kind,
        frame: 0,
        countdown: 2,
    }
}

fn particle(x: i32, y: i32, landed: bool) -> Particle {
    Particle {
        x,
        y,
        ground_y: y,
        shape_10: 4,
        shape_14: 15,
        phase: 0,
        landed,
        bounces: 3,
        color: 77,
        alpha: 0xFF,
    }
}

fn p9(frame_rate: u32) -> Pass9Input {
    Pass9Input {
        frame: F800,
        mode: OpenMode::NONE,
        frame_rate,
        low_quality: false,
        thunder_sound_starts: true,
    }
}

#[test]
fn pools_take_lowest_free_slot() {
    let mut w = Weather::new();
    assert_eq!(w.particles().capacity(), 256);
    assert_eq!(w.splashes().capacity(), 512);
    assert_eq!(w.bubbles().capacity(), 128);
    let mut p: Pool<u8> = Pool::new(3);
    assert_eq!(p.alloc(1), Some(0));
    assert_eq!(p.alloc(2), Some(1));
    assert_eq!(p.alloc(3), Some(2));
    assert_eq!(p.alloc(4), None);
    p.free(1);
    assert_eq!((p.live(), p.highest()), (2, Some(2)));
    assert_eq!(p.alloc(5), Some(1));
    let order: Vec<_> = p.iter().map(|(i, v)| (i, *v)).collect();
    assert_eq!(order, [(0, 1), (1, 5), (2, 3)]);
    p.clear();
    assert_eq!((p.live(), p.highest(), p.iter().count()), (0, None, 0));
    assert!(w.thunder);
    assert_eq!((w.snow_goal_min, w.snow_goal_n), (42, 170));
    w.pools_mut().1.alloc(rec(0, 0, 2));
    assert_eq!(w.splashes().live(), 1);
}

#[test]
fn rain_off_clears_particles_and_splashes() {
    let mut w = loaded(level(1, 0, false, false));
    w.rain_flag = true;
    w.mud_flag = true;
    (w.target, w.intensity_256) = (100, 100);
    let (p, s, b) = w.pools_mut();
    p.alloc(particle(1, 1, false));
    s.alloc(rec(1, 1, 2));
    b.alloc(rec(1, 1, 2));
    let mut seed = Seed::new(5, 0);
    let before = seed;
    // Rain off, mud on: particles and splashes go, bubbles stay.
    w.update(
        Some(LocalPlayer {
            seed: &mut seed,
            level: level(1, 0, false, true),
        }),
        &input(1),
    )
    .unwrap();
    assert_eq!((w.particles().live(), w.splashes().live()), (0, 0));
    assert_eq!(w.bubbles().live(), 1);
    assert!(!w.rain_flag && w.mud_flag);
    // Rain off: intensity := 0.0, the target is kept.
    assert_eq!((w.target, w.intensity_256), (100, 0));
    assert_eq!(seed, before, "rain off draws nothing");
    // Mud off now: bubbles cleared.
    w.update(
        Some(LocalPlayer {
            seed: &mut seed,
            level: level(1, 0, false, false),
        }),
        &input(2),
    )
    .unwrap();
    assert_eq!(w.bubbles().live(), 0);
}

#[test]
fn update_needs_player_and_runs_once_per_update_count() {
    let mut w = loaded(level(1, 0, true, true));
    assert_eq!(w.update(None, &input(1)), Err(WeatherError::Fatal(0x547)));
    let mut seed = Seed::new(99, 0);
    let lv = level(1, 0, true, true);
    w.update(
        Some(LocalPlayer {
            seed: &mut seed,
            level: lv,
        }),
        &input(5),
    )
    .unwrap();
    assert_eq!(w.mark, 5);
    let after = seed;
    let state = w.clone();
    // Same update count: a second frame changes nothing.
    w.update(
        Some(LocalPlayer {
            seed: &mut seed,
            level: lv,
        }),
        &input(5),
    )
    .unwrap();
    assert_eq!(seed, after);
    assert_eq!(w, state);
}

#[test]
fn first_rain_update_enters_phase_one_and_spawns() {
    let lv = level(1, 0, true, false);
    let mut w = loaded(lv);
    assert_eq!((w.phase, w.countdown), (0, 0));
    let mut seed = Seed::new(0x1234_5678, 0);
    let mut e = seed;
    w.update(
        Some(LocalPlayer {
            seed: &mut seed,
            level: lv,
        }),
        &input(1),
    )
    .unwrap();
    // Cycle: D, then the level-entry re-check (three raw steps), then the peak.
    let d = e.roll_range(250, 250) as u32;
    let wc = 125 + (e.step() % 375) as i32;
    let lc = 500 + (e.step() % 1_500) as i32;
    let wind = 92 + (e.step() % 71) as i32;
    let peak = e.roll_range(32, 224) as u32;
    assert_eq!((w.phase, w.length, w.countdown), (1, d, d - 1));
    assert_eq!(w.lightning_countdown, lc);
    assert_eq!((w.wind, w.wind_goal), (wind, wind));
    assert_eq!(w.peak, peak);
    let target = peak / d;
    assert_eq!(w.target, target);
    assert_eq!(w.intensity_256, target);
    // Top-up: one rain spawn per missing particle.
    for _ in 0..target {
        let x = e.roll_range(0, 800);
        let g = e.roll_range(40, 600 - 87);
        let y = e.roll_range(-20, g + 20);
        let phase = e.step() & 511;
        let ci = (e.step() % 12) as usize;
        let (_, p) = w.particles().iter().next().unwrap();
        assert_eq!((p.x, p.ground_y, p.y, p.phase), (x, g, y, phase));
        assert_eq!((p.color, p.alpha), (colors().period2[ci], 0xFF));
        assert_eq!((p.bounces, p.landed), (3, false));
        let (num, den) = (g - 40, 600 - 87);
        assert_eq!(
            (p.shape_10, p.shape_14),
            (4 + 8 * num / den, 15 + 15 * num / den)
        );
    }
    assert_eq!(w.particles().live() as u32, target);
    // Wind already on its goal; retarget countdown −1.
    assert_eq!(w.wind_countdown, wc - 1);
    assert_eq!(seed, e);
}

#[test]
fn ramp_then_phase_two_presets() {
    let lv = level(1, 0, true, false);
    let mut w = loaded(lv);
    (w.phase, w.length, w.countdown, w.peak) = (1, 250, 1, 200);
    w.wind_countdown = 50;
    let mut seed = Seed::new(3, 0);
    w.target = 0;
    w.update(
        Some(LocalPlayer {
            seed: &mut seed,
            level: lv,
        }),
        &input(1),
    )
    .unwrap();
    assert_eq!((w.countdown, w.target), (0, 200));
    w.particles.clear();
    w.target = 0;
    w.lightning_on = true;
    // Countdown 0: phase 2, D from (3000, 3000), presets: lightning off,
    // one raw step (rain mode), target := peak. Then a 200-particle top-up.
    w.update(
        Some(LocalPlayer {
            seed: &mut seed,
            level: lv,
        }),
        &input(2),
    )
    .unwrap();
    // The first update's top-up drew 200 spawns.
    let mut e = Seed::new(3, 0);
    for _ in 0..200 {
        e.roll_range(0, 800);
        let g = e.roll_range(40, 513);
        e.roll_range(-20, g + 20);
        e.step();
        e.step();
    }
    let d = e.roll_range(3_000, 3_000) as u32;
    e.step();
    assert_eq!((w.phase, w.length, w.countdown), (2, d, d - 1));
    assert!(!w.lightning_on);
    assert_eq!(w.target, 200);
    assert_eq!(w.intensity_256, 200);
    // Phase 3 ramp down: peak × countdown / D.
    (w.phase, w.length, w.countdown, w.peak) = (3, 100, 51, 200);
    w.rain_cycle(&mut seed, &lv).unwrap();
    assert_eq!(w.target, 200 * 50 / 100);
}

#[test]
fn act_load_cycle_tables() {
    let w = loaded(level(1, 0, false, false));
    assert_eq!(w.cycle_min, [7_500, 250, 3_000, 125]);
    assert_eq!(w.cycle_n, [7_500, 250, 3_000, 50]);
    assert_eq!(w.intensity_256, 0);
    assert_eq!(w.resources(), Some(&resources()));
    for act in [2, 4] {
        let w = loaded(level(if act == 4 { 109 } else { 75 }, act, false, false));
        assert_eq!(w.cycle_min[2], 9_000);
    }
    let w = loaded(level(40, 1, false, false));
    assert_eq!(w.cycle_min[2], 3_000);
}

#[test]
fn wind_retarget_and_lightning_timer() {
    let lv = level(1, 0, true, false);
    let mut w = loaded(lv);
    (w.phase, w.length, w.countdown, w.peak) = (0, 7_500, 10, 0);
    (w.wind, w.wind_goal, w.wind_countdown) = (100, 95, 1);
    (w.lightning_on, w.lightning_countdown) = (true, 2);
    let mut seed = Seed::new(11, 0);
    let mut e = seed;
    w.update(
        Some(LocalPlayer {
            seed: &mut seed,
            level: lv,
        }),
        &input(1),
    )
    .unwrap();
    assert_eq!(w.wind, 98, "step 2 toward the goal");
    let wc = 125 + (e.step() % 375) as i32;
    let goal = 92 + (e.step() % 71) as i32;
    assert_eq!((w.wind_countdown, w.wind_goal), (wc, goal));
    assert_eq!(seed, e);
    assert_eq!((w.lightning_countdown, w.lightning_trigger), (1, 0));
    (w.wind, w.wind_goal) = (96, 95);
    w.update(
        Some(LocalPlayer {
            seed: &mut seed,
            level: lv,
        }),
        &input(2),
    )
    .unwrap();
    assert_eq!(w.wind, 95, "stops on the goal");
    assert_eq!((w.lightning_countdown, w.lightning_trigger), (0, 1));
}

#[test]
fn live_particles_move_before_the_rain_cycle() {
    // With particles live, the update moves them and counts
    // the move in `F`; with none live there is no move.
    let lv = level(1, 0, true, false);
    let mut w = loaded(lv);
    w.particles.alloc(particle(5, 5, false));
    let mut seed = Seed::new(1, 0);
    w.update(
        Some(LocalPlayer {
            seed: &mut seed,
            level: lv,
        }),
        &input(1),
    )
    .unwrap();
    assert_eq!(w.move_counter, 1);
    let mut empty = loaded(lv);
    let mut seed = Seed::new(1, 0);
    empty
        .update(
            Some(LocalPlayer {
                seed: &mut seed,
                level: lv,
            }),
            &input(1),
        )
        .unwrap();
    assert_eq!(empty.move_counter, 0);
}

/// A rain weather with wind 0 (`Wc` = 1, `Ws` = 0) and intensity 0
/// (f = float32(0.85)), one particle with +0x14 = 20 (A = 17).
fn mover(p: Particle) -> Weather {
    let mut w = loaded(level(1, 0, true, false));
    assert_eq!(sine_table()[128], 1.0);
    assert_eq!(sine_table()[0], 0.0);
    w.wind = 0;
    w.intensity_256 = 0;
    w.particles.alloc(Particle { shape_14: 20, ..p });
    w
}

fn step(w: &mut Weather, delta: (i32, i32)) {
    let mut seed = Seed::new(3, 0);
    let input = UpdateInput {
        camera_delta: delta,
        ..input(1)
    };
    w.move_particles(&mut seed, &input).unwrap();
}

#[test]
fn particle_move_rain_vectors() {
    // f = float32(0 × 0.15 + 0.85); A = trunc(20 × f) = 17; ux = trunc(
    // Wc(0) × 17) = 17, uy = trunc(Ws(0) × 17) = 0. y += uy − dY;
    // x += ux − dX.
    let mut w = mover(particle(100, 50, false));
    w.particles.free(0);
    w.particles.alloc(Particle {
        ground_y: 100,
        shape_14: 20,
        ..particle(100, 50, false)
    });
    step(&mut w, (3, 2));
    let p = *w.particles.get(0).unwrap();
    assert_eq!((p.x, p.y, p.landed, p.bounces), (114, 48, false, 3));
    assert_eq!(w.move_counter, 1);
    // x ≥ W wraps once down by W, x < 0 once up by W (W = 800).
    for (x, want) in [(795, 9), (-20, 794), (786, 0)] {
        let mut w = mover(Particle {
            ground_y: 100,
            ..particle(x, 50, false)
        });
        step(&mut w, (3, 0));
        assert_eq!(w.particles.get(0).unwrap().x, want, "x {x}");
    }
    // F counts every call and never resets.
    step(&mut w, (0, 0));
    assert_eq!(w.move_counter, 2);
}

#[test]
fn particle_move_landing_bounces_then_frees_and_respawns() {
    // y > ground y → landed; with bounces left: ux := 0, +0x14 := 0,
    // bounce count − 1 (x moves by −dX only).
    let mut w = mover(Particle {
        ground_y: 100,
        ..particle(100, 99, false)
    });
    step(&mut w, (3, -5));
    let p = *w.particles.get(0).unwrap();
    assert_eq!((p.y, p.landed, p.bounces, p.shape_14), (104, true, 2, 0));
    assert_eq!(p.x, 97);
    // No bounces left: the slot is freed; with live < target one particle
    // spawns on the player seed (the lowest free slot: the same one) and
    // is moved in the same loop with the freed particle's ux (17).
    let mut w = mover(Particle {
        ground_y: 100,
        bounces: 0,
        landed: true,
        ..particle(100, 99, true)
    });
    w.target = 1;
    let mut seed = Seed::new(3, 0);
    let mut probe = seed;
    let spawn_x = probe.roll_range(0, 800);
    w.move_particles(&mut seed, &input(1)).unwrap();
    assert_eq!(w.particles.live(), 1);
    let p = *w.particles.get(0).unwrap();
    assert_eq!((p.bounces, p.landed), (3, false));
    assert_eq!(p.x, (spawn_x + 17).rem_euclid(800));
    // At the target: freed and not replaced; no draw on the seed.
    let mut w = mover(Particle {
        ground_y: 100,
        bounces: 0,
        landed: true,
        ..particle(100, 99, true)
    });
    w.target = 0;
    let mut seed = Seed::new(3, 0);
    w.move_particles(&mut seed, &input(1)).unwrap();
    assert_eq!(w.particles.live(), 0);
    assert_eq!(seed, Seed::new(3, 0));
}

#[test]
fn particle_move_snow_sway_uses_the_counter() {
    // Snow: f = max(|Wc|, 0.25) = 1, A = 20, ux = 20; a particle that has
    // not landed adds trunc(2 × Ws(F × 512 / 25 + phase)).
    let mut w = mover(particle(100, 50, false));
    w.snow_mode = true;
    w.move_counter = 10; // 5120 / 25 = 204
    let want = (2.0 * (204.0f64 * std::f64::consts::PI / 256.0).sin()) as i32;
    assert_eq!(want, 1);
    step(&mut w, (0, 0));
    assert_eq!(w.particles.get(0).unwrap().x, 100 + 20 + want);
    assert_eq!(w.move_counter, 11);
    // A landed snow flake does not sway (the bounce zeroed ux).
    let mut w = mover(Particle {
        ground_y: 100,
        ..particle(100, 99, false)
    });
    w.snow_mode = true;
    w.move_counter = 10;
    step(&mut w, (0, -5));
    assert_eq!(w.particles.get(0).unwrap().x, 100);
    // The product wraps at 32 bits before the divide.
    let mut w = mover(particle(100, 50, false));
    w.snow_mode = true;
    w.move_counter = 0x0200_0000 + 50; // × 512 wraps to 50 × 512
    let arg = (50u32 * 512) / 25;
    let want = (2.0 * (f64::from(arg & 511) * std::f64::consts::PI / 256.0).sin()) as i32;
    step(&mut w, (0, 0));
    assert_eq!(w.particles.get(0).unwrap().x, 100 + 20 + want);
}

/// A seed whose rain / snow spawn at `H` 600 draws ground y `g`.
fn seed_with_ground(g: i32) -> Seed {
    (1..)
        .map(|lo| Seed::new(lo, 0))
        .find(|s| {
            let mut s = *s;
            s.roll_range(0, 800);
            s.roll_range(40, 513) == g
        })
        .unwrap()
}

#[test]
fn spawn_shape_values_and_snow_spawn() {
    let lv = level(1, 0, true, false);
    // Test vectors: H 600, g = 296. Rain: +0x10 = 4 + ⌊2048 / 513⌋ = 7,
    // +0x14 = 15 + ⌊3840 / 513⌋ = 22.
    let mut w = loaded(lv);
    let mut seed = seed_with_ground(296);
    let mut e = seed;
    assert!(w.spawn_particle(&mut seed, &input(1)).unwrap());
    let (_, p) = w.particles().iter().next().unwrap();
    assert_eq!((p.ground_y, p.shape_10, p.shape_14), (296, 7, 22));
    for _ in 0..5 {
        e.step();
    }
    assert_eq!(seed, e, "rain: x, g, y, phase, color");
    // Snow, no bump: s = 1792 / 513 = 3, +0x14 = 8 + 13 = 21; one more raw
    // step after the phase step, no color step; bounce count 1; color and
    // alpha from the snow colour (day period 2, video mode 3: S[(36 / 8) / 4]).
    let mut w = loaded(lv);
    w.snow_mode = true;
    let mut seed = seed_with_ground(296);
    let mut e = seed;
    assert!(w.spawn_particle(&mut seed, &input(1)).unwrap());
    let (_, p) = w.particles().iter().next().unwrap();
    assert_eq!((p.shape_10, p.shape_14, p.bounces), (3, 21, 1));
    assert_eq!((p.color, p.alpha), (colors().snow[1], 80));
    for _ in 0..5 {
        e.step();
    }
    assert_eq!(seed, e);
    // Size bump: s + 1.
    let mut w = loaded(lv);
    (w.snow_mode, w.size_bump) = (true, true);
    let mut seed = seed_with_ground(296);
    w.spawn_particle(&mut seed, &input(1)).unwrap();
    assert_eq!(w.particles().iter().next().unwrap().1.shape_10, 4);
    for (period, table, alpha) in [
        (0, colors().period0, 0x7F),
        (1, colors().period13, 0xFF),
        (3, colors().period13, 0xFF),
    ] {
        let mut w = loaded(lv);
        let mut seed = Seed::new(42, 0);
        let mut e = seed;
        let inp = UpdateInput {
            day_period: period,
            ..input(1)
        };
        assert!(w.spawn_particle(&mut seed, &inp).unwrap());
        for _ in 0..4 {
            e.step();
        }
        let i = (e.step() % 12) as usize;
        let (_, p) = w.particles().iter().next().unwrap();
        assert_eq!((p.color, p.alpha), (table[i], alpha));
    }
}

#[test]
fn snow_color_by_period_and_video_mode() {
    let c = colors();
    // Test vector: s = 5, day period 2, video mode 3 → i = 7, S[7 / 4].
    assert_eq!(snow_color(&c, 5, 2, 3), Ok((c.snow[1], 80)));
    assert_eq!(snow_color(&c, 5, 0, 6), Ok((c.snow[7], 200)));
    assert_eq!(snow_color(&c, 5, 1, 1), Ok((c.snow[3], 160)));
    assert_eq!(snow_color(&c, 5, 3, 2), Ok((c.snow[3], 160)));
    // Other video modes: S'[i] whatever the period.
    assert_eq!(snow_color(&c, 5, 2, 4), Ok((c.snow_alt[7], 80)));
    assert_eq!(snow_color(&c, 7, 0, 0), Ok((c.snow_alt[10], 200)));
    assert_eq!(snow_color(&c, 5, 4, 3), Err(WeatherError::Fatal(0x356)));
}

#[test]
fn color_tables_from_the_ramps() {
    use d2_formats::palette::{Palette, Rgb};
    // A palette whose entry j is (j, j, j) except a few probe colors.
    let mut colors: Vec<Rgb> = (0..=255u8).map(|j| Rgb { r: j, g: j, b: j }).collect();
    colors[200] = Rgb {
        r: 25,
        g: 50,
        b: 25,
    };
    colors[201] = Rgb { r: 3, g: 8, b: 3 };
    let pal = Palette {
        colors: colors.try_into().unwrap(),
    };
    let t = ColorTables::build(&pal);
    // Worked vector, i = 11: rain period 0 v = 98 − 73 = 25 → nearest(25, 50,
    // 25); rain period 2 v = 3 → nearest(3, 8, 3).
    assert_eq!(t.period0[11], 200);
    assert_eq!(t.period2[11], 201);
    assert_eq!(t.period0[0], nearest(&pal, 98, 123, 98));
    assert_eq!(t.period13[11], nearest(&pal, 9, 19, 9));
    assert_eq!(t.snow[11], 120 + 73);
    assert_eq!(t.snow_alt[0], 170);
}

#[test]
fn water_floor_rain_off_draws_once() {
    // Test vector: water floor drawn, rain off → 1 draw, no spawn.
    let mut w = loaded(level(1, 0, false, false));
    let mut ctx = FloorContext::default();
    ctx.begin_frame(100, &w, false);
    assert!(!ctx.splash && !ctx.bubble);
    let mut seed = Seed::new(17, 0);
    let mut e = seed;
    w.water_floor(&ctx, 400, 300, &mut seed);
    e.step();
    assert_eq!(seed, e);
    assert_eq!((w.splashes().live(), w.bubbles().live()), (0, 0));
}

#[test]
fn floor_context_gates() {
    let mut w = Weather::new();
    let mut ctx = FloorContext::default();
    // k = 0: no splash, but last_s still advances.
    ctx.begin_frame(25, &w, true);
    assert!(!ctx.splash, "k = 0");
    assert_eq!(ctx.last_s, 25, "last_s advances whatever k is");
    assert!(!ctx.bubble, "25 is not > 0 + 25");
    ctx.begin_frame(26, &w, true);
    assert!(ctx.bubble);
    assert_eq!(ctx.last_b, 26);
    w.intensity_256 = 255;
    ctx.begin_frame(28, &w, true);
    assert!(
        !ctx.splash && !ctx.bubble,
        "28 is not > 25 + 3; flags cleared"
    );
    assert_eq!(ctx.k, 996);
    ctx.begin_frame(29, &w, true);
    assert!(ctx.splash);
    assert_eq!(ctx.last_s, 29);
    ctx.begin_frame(52, &w, false);
    assert!(!ctx.bubble, "no Mud");
    ctx.begin_frame(52, &w, true);
    assert!(ctx.bubble);
}

#[test]
fn splash_threshold_vectors() {
    let mut w = Weather::new();
    for (target, k) in [
        (32, 125),
        (224, 875),
        (255, 996),
        (128, 500),
        (0, 0),
        (256, 1000),
    ] {
        w.intensity_256 = target;
        assert_eq!(w.splash_threshold(), k, "target {target}");
    }
    // Target 128: r = 499 spawns, r = 500 does not.
    w.intensity_256 = 128;
    let ctx = FloorContext {
        k: w.splash_threshold(),
        splash: true,
        ..Default::default()
    };
    for (r, spawns) in [(499, true), (500, false)] {
        let seed0 = (1..)
            .map(|lo| Seed::new(lo, 0))
            .find(|s| s.clone().roll_range(0, 1_000) == r)
            .unwrap();
        let mut w = Weather::new();
        let mut seed = seed0;
        w.water_floor(&ctx, 10, 20, &mut seed);
        assert_eq!(w.splashes().live() == 1, spawns, "r = {r}");
    }
}

/// A seed whose first `roll_range(0, 80)` gives `a`.
fn seed_with_a(a: i32) -> Seed {
    (1..)
        .map(|lo| Seed::new(lo, 0))
        .find(|s| s.clone().roll_range(0, 80) == a)
        .unwrap()
}

#[test]
fn splash_spawn_a_zero() {
    // Test vector: a = 0 → b = 0, no x draw, x = X, y = Y.
    let mut w = Weather::new();
    let mut seed = seed_with_a(0);
    let mut e = seed;
    let i = w.spawn_splash(400, 300, &mut seed).unwrap();
    e.step();
    let kind = 2 + (e.step() & 1) as u8;
    assert_eq!(seed, e, "two draws: a and the kind step");
    assert_eq!(*w.splashes().get(i).unwrap(), rec(400, 300, kind));
}

#[test]
fn splash_and_bubble_spawn_positions() {
    let mut w = Weather::new();
    let mut seed = seed_with_a(30);
    let mut e = seed;
    w.spawn_splash(100, 200, &mut seed).unwrap();
    assert_eq!(e.roll_range(0, 80), 30);
    let b = 80 - 2 * 10;
    let dx = e.roll_range(-b, 2 * b);
    let kind = 2 + (e.step() & 1) as u8;
    let r = *w.splashes().get(0).unwrap();
    assert_eq!(r, rec(100 + dx, 230, kind));
    assert_eq!(seed, e);

    let mut seed = seed_with_a(50);
    let mut e = seed;
    w.spawn_bubble(100, 200, &mut seed).unwrap();
    e.roll_range(0, 80);
    let dx = e.roll_range(-60, 120);
    assert_eq!(*w.bubbles().get(0).unwrap(), rec(100 + dx, 220, 2));
    assert_eq!(seed, e, "bubble: two draws, no kind step");

    // Full pool: nothing, no draw.
    for _ in 1..BUBBLE_SLOTS {
        w.bubbles.alloc(rec(0, 0, 2));
    }
    let before = seed;
    assert_eq!(w.spawn_bubble(1, 1, &mut seed), None);
    assert_eq!(seed, before);
}

#[test]
fn water_floor_spawns_when_gated() {
    let mut w = Weather::new();
    let ctx = FloorContext {
        bubble: true,
        ..Default::default()
    };
    // Find a seed with r < 100.
    let seed0 = (1..)
        .map(|lo| Seed::new(lo, 0))
        .find(|s| s.clone().roll_range(0, 1_000) < 100)
        .unwrap();
    let mut seed = seed0;
    w.water_floor(&ctx, 10, 20, &mut seed);
    assert_eq!((w.splashes().live(), w.bubbles().live()), (0, 1));
    let mut e = seed0;
    e.roll_range(0, 1_000);
    let mut probe = Weather::new();
    probe.spawn_bubble(10, 20, &mut e);
    assert_eq!(seed, e);
    assert_eq!(probe.bubbles().get(0), w.bubbles().get(0));
}

#[test]
fn pool_update_moves_and_ages() {
    let lv = level(1, 0, false, true);
    let mut w = loaded(lv);
    w.bubbles.alloc(rec(100, 100, 2)); // bubble3: 7 frames
    let mut seed = Seed::new(1, 0);
    let mut inp = input(1);
    inp.camera_delta = (4, -2);
    w.update(
        Some(LocalPlayer {
            seed: &mut seed,
            level: lv,
        }),
        &inp,
    )
    .unwrap();
    let r = *w.bubbles().get(0).unwrap();
    assert_eq!((r.x, r.y, r.frame, r.countdown), (96, 102, 0, 1));
    inp.update_count = 2;
    w.update(
        Some(LocalPlayer {
            seed: &mut seed,
            level: lv,
        }),
        &inp,
    )
    .unwrap();
    let r = *w.bubbles().get(0).unwrap();
    assert_eq!((r.x, r.y, r.frame, r.countdown), (92, 104, 1, 2));
    // 6 more frames (12 updates) → frame 7 ≥ 7: freed.
    for c in 3..=14 {
        inp.update_count = c;
        w.update(
            Some(LocalPlayer {
                seed: &mut seed,
                level: lv,
            }),
            &inp,
        )
        .unwrap();
    }
    assert_eq!(w.bubbles().live(), 0);
}

#[test]
fn pass4_order_and_bounds() {
    let mut w = Weather::new();
    w.splashes.alloc(rec(10, 10, 2));
    w.splashes.alloc(rec(799, 552, 3));
    w.splashes.alloc(rec(800, 10, 2)); // x = W: out
    w.splashes.alloc(rec(10, 553, 2)); // y = H − 47: out
    w.splashes.alloc(rec(10, -1, 2)); // y < 0: out
    w.bubbles.alloc(rec(20, 20, 2));
    w.splashes.free(0);
    let d = w.pass4(F800, mode(0), 0);
    let got: Vec<_> = d.iter().map(|d| (d.pool, d.slot, d.x, d.y)).collect();
    assert_eq!(
        got,
        [
            (EnvPool::Splashes, 1, 799, 552),
            (EnvPool::Bubbles, 0, 20, 20)
        ]
    );
    assert_eq!((d[0].kind, d[0].frame), (3, 0));
    assert_eq!(
        EnvPool::Splashes.cel_file(d[0].kind),
        "DATA\\GLOBAL\\UncompOverlays\\Rain4"
    );
    assert_eq!(
        EnvPool::Bubbles.cel_file(2),
        "DATA\\GLOBAL\\UncompOverlays\\bubble3"
    );
    assert_eq!(
        (EnvPool::Splashes.major(), EnvPool::Bubbles.major()),
        (0, 1)
    );
    assert_eq!((PoolDraw::LIGHT, PoolDraw::DRAW_MODE), (-1, 3));
    // Open mode 1: [0, 400), shiftX −200 added.
    assert_eq!(span(F800, mode(1)), (0, 400));
    assert_eq!(span(F800, mode(2)), (400, 800));
    assert_eq!(span(F800, mode(3)), (0, 800));
    let mut w = Weather::new();
    w.splashes.alloc(rec(150, 10, 2)); // −50: out
    w.splashes.alloc(rec(300, 10, 2)); // 100: in
    w.splashes.alloc(rec(600, 10, 2)); // 400 = R: out
    let d = w.pass4(F800, mode(1), -200);
    let got: Vec<_> = d.iter().map(|d| (d.slot, d.x)).collect();
    assert_eq!(got, [(1, 100)]);
}

#[test]
fn flash_needs_ten_frames_per_second() {
    // Test vector: frame rate 9 / 10, open mode 0, 800 × 600.
    let lv = level(1, 0, false, false);
    for (fps, flash) in [(9, false), (10, true)] {
        let mut w = Weather::new();
        w.start_lightning(true);
        let mut seed = Seed::new(1, 0);
        let out = w
            .pass9(
                Some(LocalPlayer {
                    seed: &mut seed,
                    level: lv,
                }),
                &p9(fps),
            )
            .unwrap();
        let want: &[SkyDraw] = if flash {
            &[SkyDraw::Flash {
                x0: 0,
                y0: 0,
                x1: 800,
                y1: 553,
            }]
        } else {
            &[]
        };
        assert_eq!(out.draws, want);
        assert_eq!((w.lightning_phase, w.lightning_countdown), (1, 3));
    }
    let mut w = Weather::new();
    assert_eq!(w.pass9(None, &p9(30)), Err(WeatherError::Fatal(0x573)));
}

#[test]
fn thunder_first_strike_silent() {
    let lv = level(1, 0, false, false);
    let mut w = Weather::new();
    w.start_lightning(false);
    assert!(w.lightning_on && !w.thunder && w.lightning_trigger == 1);
    w.lightning_phase = 1;
    let mut seed = Seed::new(8, 0);
    let mut e = seed;
    let out = w
        .pass9(
            Some(LocalPlayer {
                seed: &mut seed,
                level: lv,
            }),
            &p9(30),
        )
        .unwrap();
    assert_eq!(w.lightning_countdown, e.roll_range(500, 1_500));
    assert_eq!(seed, e);
    assert_eq!(out.thunder, None);
    assert!(w.thunder, "a cleared thunder flag is set instead");
    assert_eq!(w.lightning_phase, 0);
    // Second strike: sound 202 with volume and two position rolls.
    w.lightning_phase = 1;
    w.lightning_trigger = 1;
    let out = w
        .pass9(
            Some(LocalPlayer {
                seed: &mut seed,
                level: lv,
            }),
            &p9(30),
        )
        .unwrap();
    // Thunder: the delay, then y before x.
    let cd = e.roll_range(500, 1_500);
    let delay = e.roll_range(25, 50);
    let y = e.roll_range(-200, 400);
    let x = e.roll_range(-200, 400);
    assert_eq!(w.lightning_countdown, cd);
    assert_eq!(
        out.thunder,
        Some(Thunder {
            sound: 202,
            delay,
            position: Some((x, y))
        })
    );
    assert_eq!(seed, e);
    // Sound not started: no position rolls.
    w.lightning_phase = 1;
    w.lightning_trigger = 1;
    let inp = Pass9Input {
        thunder_sound_starts: false,
        ..p9(30)
    };
    let out = w
        .pass9(
            Some(LocalPlayer {
                seed: &mut seed,
                level: lv,
            }),
            &inp,
        )
        .unwrap();
    e.roll_range(500, 1_500);
    assert_eq!(out.thunder.unwrap().delay, e.roll_range(25, 50));
    assert_eq!(seed, e);
}

#[test]
fn particles_draw_only_landed_drops() {
    let lv = level(1, 0, true, false);
    let mut w = Weather::new();
    w.particles.alloc(particle(10, 10, true));
    w.particles.alloc(particle(800, 10, true)); // out of range
    let mut seed = Seed::new(1, 0);
    let out = w
        .pass9(
            Some(LocalPlayer {
                seed: &mut seed,
                level: lv,
            }),
            &p9(30),
        )
        .unwrap();
    assert_eq!(
        out.draws,
        [SkyDraw::Line {
            x0: 10,
            y0: 10,
            x1: 10,
            y1: 10,
            color: 77,
            alpha: 0xFF
        }]
    );
    // Low quality: nothing.
    let lq = Pass9Input {
        low_quality: true,
        ..p9(30)
    };
    let out = w
        .pass9(
            Some(LocalPlayer {
                seed: &mut seed,
                level: lv,
            }),
            &lq,
        )
        .unwrap();
    assert!(out.draws.is_empty());
}

#[test]
fn falling_drops_and_snow_lines() {
    let lv = level(1, 0, true, false);
    let pass = |w: &mut Weather| {
        let mut seed = Seed::new(1, 0);
        w.pass9(
            Some(LocalPlayer {
                seed: &mut seed,
                level: lv,
            }),
            &p9(30),
        )
    };
    let t = crate::rules::lighting::overrides::sine_table();
    // Wind 128: u = trunc(Wt[256] × len), v = trunc(Wt[128] × len).
    let mut w = Weather::new();
    w.wind = 128;
    let mut p = particle(20, 20, false);
    (p.ground_y, p.shape_10) = (100, 7);
    w.particles.alloc(p);
    let u = (f64::from(t[256]) * 7.0) as i32;
    let v = (f64::from(t[128]) * 7.0) as i32;
    assert_eq!((u, v), (0, 7));
    let out = pass(&mut w).unwrap();
    let want = SkyDraw::Line {
        x0: 20,
        y0: 20,
        x1: 20 + u,
        y1: 20 + v,
        color: 77,
        alpha: 0xFF,
    };
    assert_eq!(out.draws, [want]);
    // Cut at the ground: v > g − y → u := (g − y) × u / v, v := g − y.
    let mut w = Weather::new();
    w.wind = 100;
    let mut p = particle(20, 20, false);
    (p.ground_y, p.shape_10) = (23, 10);
    w.particles.alloc(p);
    let u = (f64::from(t[228]) * 10.0) as i32;
    let v = (f64::from(t[100]) * 10.0) as i32;
    assert!(v > 3 && u != 0);
    let out = pass(&mut w).unwrap();
    assert_eq!(
        out.draws,
        [SkyDraw::Line {
            x0: 20,
            y0: 20,
            x1: 20 + 3 * u / v,
            y1: 23,
            color: 77,
            alpha: 0xFF
        }]
    );
    // Either end in range draws: origin out (y < 0), end in.
    let mut w = Weather::new();
    w.wind = 128;
    let mut p = particle(20, -3, false);
    (p.ground_y, p.shape_10) = (100, 7);
    w.particles.alloc(p);
    assert_eq!(pass(&mut w).unwrap().draws.len(), 1);
    let mut w = Weather::new();
    w.wind = 128;
    let mut p = particle(20, -30, false);
    (p.ground_y, p.shape_10) = (100, 7);
    w.particles.alloc(p);
    assert!(pass(&mut w).unwrap().draws.is_empty());
    // Snow: size s draws T[s] then T[s + 1]; s outside 0–7 draws nothing.
    let mut w = Weather::new();
    w.snow_mode = true;
    let mut p = particle(50, 60, false);
    p.shape_10 = 7;
    w.particles.alloc(p);
    let mut q = particle(70, 60, false);
    q.shape_10 = 8;
    w.particles.alloc(q);
    let out = pass(&mut w).unwrap();
    let l = |a: [i32; 4]| SkyDraw::Line {
        x0: 50 + a[0],
        y0: 60 + a[1],
        x1: 50 + a[2],
        y1: 60 + a[3],
        color: 77,
        alpha: 0xFF,
    };
    assert_eq!(out.draws, [l([2, 1, -2, -1]), l([2, -1, -2, 2])]);
    assert_eq!(SNOW_LINES[0], [0, 0, 0, 1]);
    assert_eq!(SNOW_LINES[3], [1, 0, -1, 1]);
}

#[test]
fn act5_lock_levels() {
    let lv = level(120, 4, true, false);
    let mut w = Weather::new();
    w.particles.alloc(particle(1, 1, true));
    w.lightning_on = true;
    let mut seed = Seed::new(5, 0);
    let mut e = seed;
    w.level_entry(0, &mut seed, &lv).unwrap();
    for _ in 0..4 {
        e.step();
    }
    assert_eq!(
        seed, e,
        "weather initialization (3 steps) + 1 raw step, no preset draws"
    );
    assert!(w.snow_mode && w.snow_lock && w.size_bump);
    assert_eq!((w.peak, w.target, w.wind, w.wind_goal), (256, 256, 28, 28));
    assert_eq!((w.snow_goal_min, w.snow_goal_n), (28, 28));
    assert!(!w.lightning_on);
    assert_eq!(w.particles().live(), 0, "mode changed: cleared");
    // Locked and phase ≠ 2: the cycle forces phase 2 and the presets.
    w.phase = 0;
    w.target = 0;
    w.rain_cycle(&mut seed, &lv).unwrap();
    assert_eq!((w.phase, w.target), (2, 256));
    // The unlock switch is never set: every locked cycle off phase 2 is forced.
    w.phase = 3;
    w.target = 0;
    w.rain_cycle(&mut seed, &lv).unwrap();
    assert_eq!((w.phase, w.target), (2, 256));
    // At phase 2 the cycle runs; its countdown ends: p = 3, the locked
    // entry keeps phase 2, but this call ramps with p = 3.
    (w.countdown, w.peak) = (0, 200);
    (w.cycle_min, w.cycle_n) = (CYCLE_MIN, CYCLE_N);
    let mut e = seed;
    w.rain_cycle(&mut seed, &lv).unwrap();
    let d = e.roll_range(125, 50) as u32;
    assert_eq!(seed, e, "the locked entry draws nothing");
    assert_eq!((w.phase, w.length, w.countdown), (2, d, d - 1));
    assert_eq!(w.target, 200 * (d - 1) / d);
    // p > 3: fatal 0x12A.
    assert_eq!(
        w.phase_entry(4, &mut seed, &lv),
        Err(WeatherError::Fatal(0x12A))
    );
}

#[test]
fn act5_snow_off_roll_and_leaving_act5() {
    let lv = level(110, 4, true, false);
    // A seed whose 4th step mod 100 < 25.
    let seed0 = (1..)
        .map(|lo| Seed::new(lo, 0))
        .find(|s| {
            let mut s = *s;
            (0..3).for_each(|_| {
                s.step();
            });
            s.step() % 100 < 25
        })
        .unwrap();
    let mut w = Weather::new();
    w.snow_mode = true;
    w.particles.alloc(particle(1, 1, true));
    let mut seed = seed0;
    w.level_entry(0, &mut seed, &lv).unwrap();
    assert!(!w.snow_mode && !w.size_bump);
    assert_eq!(w.particles().live(), 0);
    // With arg ≠ 0 the same roll keeps snow on.
    let mut w = Weather::new();
    let mut seed = seed0;
    w.level_entry(1, &mut seed, &lv).unwrap();
    assert!(w.snow_mode && !w.snow_lock);
    // Leaving act V: snow off, lightning off, the wind and lightning reset.
    w.lightning_on = true;
    w.particles.alloc(particle(1, 1, true));
    let mut e = seed;
    w.level_entry(0, &mut seed, &level(1, 0, true, false))
        .unwrap();
    let wc = 125 + (e.step() % 375) as i32;
    let lc = 500 + (e.step() % 1_500) as i32;
    let wind = 92 + (e.step() % 71) as i32;
    assert_eq!(seed, e);
    assert_eq!(
        (w.wind_countdown, w.lightning_countdown, w.wind),
        (wc, lc, wind)
    );
    assert!(!w.snow_mode && !w.lightning_on);
    assert_eq!(w.particles().live(), 0);
}

#[test]
fn level_presets_snow_levels() {
    let mut w = Weather::new();
    w.snow_mode = true;
    let mut seed = Seed::new(9, 0);
    let mut e = seed;
    w.level_presets(&mut seed, &level(117, 4, true, false));
    assert_eq!(w.peak, e.roll_range(40, 112) as u32);
    assert_eq!(
        (w.snow_goal_min, w.snow_goal_n, w.target),
        (170, 56, w.peak)
    );
    w.level_presets(&mut seed, &level(111, 4, true, false));
    assert_eq!(w.peak, e.roll_range(32, 56) as u32);
    assert_eq!((w.snow_goal_min, w.snow_goal_n), (42, 170));
    // Other level in snow mode: no raw step.
    let before = seed;
    w.level_presets(&mut seed, &level(115, 4, true, false));
    assert_eq!(seed, before);
}

#[test]
fn locked_rain_cycle_becomes_phase_2_and_then_keeps_it() {
    let lv = level(1, 0, true, false);
    // Snow lock set and the phase not 2: the phase becomes 2 and the level
    // presets run instead of the cycle (no countdown or length change).
    let mut w = loaded(lv);
    w.snow_lock = true;
    w.phase = 1;
    w.countdown = 77;
    let mut seed = Seed::new(5, 0);
    w.rain_cycle(&mut seed, &lv).unwrap();
    assert_eq!((w.phase, w.countdown), (2, 77));
    // Locked in phase 2 with the countdown run out: the cycle runs with
    // p = 3 (draws D, phase entry 3 does nothing under the lock), the
    // stored phase stays 2 and the ramp uses p = 3: target = peak × countdown / D.
    let mut w = loaded(lv);
    w.snow_lock = true;
    w.phase = 2;
    w.countdown = 0;
    w.peak = 100;
    let mut probe = Seed::new(5, 0);
    let d = probe.roll_range(w.cycle_min[3], w.cycle_n[3]) as u32;
    let mut seed = Seed::new(5, 0);
    w.rain_cycle(&mut seed, &lv).unwrap();
    assert_eq!(w.phase, 2);
    assert_eq!((w.length, w.countdown), (d, d - 1));
    assert_eq!(w.target, 100 * (d - 1) / d);
    assert_eq!(seed, probe);
}

/// A sound layer stand-in: the handle it returns and what it was asked.
#[derive(Debug)]
struct Sounds {
    handle: u32,
    calls: Vec<(u16, i32)>,
    positions: Vec<(u32, i32, i32)>,
}

impl ThunderSound for Sounds {
    fn request(&mut self, id: u16, delay: i32) -> u32 {
        self.calls.push((id, delay));
        self.handle
    }
    fn set_position(&mut self, h: u32, x: i32, y: i32) {
        self.positions.push((h, x, y));
    }
}

#[test]
fn thunder_requests_202_with_the_delay_and_places_it_y_first() {
    let lv = level(1, 0, false, false);
    let strike = |w: &mut Weather, seed: &mut Seed, s: &mut Sounds| {
        w.lightning_on = true;
        w.lightning_phase = 1;
        w.lightning_trigger = 1;
        w.thunder = true;
        w.pass9_with(
            Some(LocalPlayer { seed, level: lv }),
            &Pass9Input {
                thunder_sound_starts: false,
                ..p9(30)
            },
            Some(s),
        )
        .unwrap()
    };
    let mut w = Weather::new();
    let mut seed = Seed::new(0x1234_5678, 9);
    let mut e = seed;
    let mut s = Sounds {
        handle: 7,
        calls: Vec::new(),
        positions: Vec::new(),
    };
    let out = strike(&mut w, &mut seed, &mut s);
    e.roll_range(500, 1_500);
    let delay = e.roll_range(25, 50);
    let y = e.roll_range(-200, 400);
    let x = e.roll_range(-200, 400);
    assert_eq!(s.calls, [(202, delay)]);
    assert_eq!(s.positions, [(7, x, y)]);
    assert_eq!(out.thunder.unwrap().position, Some((x, y)));
    assert_eq!(seed, e);
    // No handle (the hook's answer, not the input flag): no position draws.
    s.handle = 0;
    let out = strike(&mut w, &mut seed, &mut s);
    e.roll_range(500, 1_500);
    let delay = e.roll_range(25, 50);
    assert_eq!(s.calls[1], (202, delay));
    assert_eq!(s.positions.len(), 1);
    assert_eq!(out.thunder.unwrap().position, None);
    assert_eq!(seed, e);
}

#[test]
fn first_act_load_adds_the_one_time_wind_init() {
    let lv = level(2, 0, true, false);
    let steps = |w: &mut Weather| {
        let mut s = Seed::new(7, 0);
        w.act_load(
            LocalPlayer {
                seed: &mut s,
                level: lv,
            },
            resources(),
        )
        .unwrap();
        let mut t = Seed::new(7, 0);
        (0..32).find(|_| {
            t.step();
            t == s
        })
    };
    let mut w = Weather::new();
    // The level-entry re-check outside act V: three steps; the one-time init: three more.
    assert_eq!(steps(&mut w), Some(5));
    // Later act loads of the same client run only the level entry.
    assert_eq!(steps(&mut w), Some(2));
}
