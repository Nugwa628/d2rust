//! Unit tests of the light quality, the environment, the overrides and the
//! light sources, from synthetic worked vectors.

use d2_sim::rng::Seed;

use super::environment::{
    act_index, room_ambient, Ambient, EclipseTrigger, EnvError, Environment, Period, PeriodTables,
    TsvError, ECLIPSE_SET, ENV_PERIODS_TSV, SPEED,
};
use super::overrides::{den_light_points, sine_table, OverrideError, Overrides, UniqueEventEffect};
use super::quality::{DrawRateMeter, LightOptions, Quality, QualityOption};
use super::sources::{self, LightRequest, MonsterLightInput, ObjectLight};

fn tables() -> PeriodTables {
    PeriodTables::builtin().expect("env-periods.tsv parses")
}

/// A normal-table environment at `deg` degrees (index found by start).
fn env_at(t: &PeriodTables, index: i32, deg: i32) -> Environment {
    let mut e = Environment::new(t, 0);
    e.index = index;
    e.ticks = SPEED * deg;
    e
}

// ---- the light-level ramp ----

#[test]
fn options_menu_mapping() {
    assert_eq!(
        LightOptions::default(),
        LightOptions {
            low_quality: false,
            missile_lights: true
        }
    );
    let f = |o| {
        let l = LightOptions::from_menu(o);
        (l.low_quality, l.missile_lights)
    };
    assert_eq!(f(QualityOption::Low), (true, false));
    assert_eq!(f(QualityOption::Medium), (false, false));
    assert_eq!(f(QualityOption::High), (false, true));
}

#[test]
fn quality_bands() {
    let hi = LightOptions::from_menu(QualityOption::High);
    // From q = 0, far past the hysteresis.
    for (d, want) in [
        (-1, 0),
        (9, 0),
        (10, 1),
        (12, 1),
        (13, 1),
        (15, 1),
        (16, 2),
        (60, 2),
    ] {
        let mut q = Quality::default();
        assert_eq!(q.update(hi, d, 10_000), want, "D = {d}");
    }
    // Low quality only: 0 without a rate test.
    let low = LightOptions::from_menu(QualityOption::Low);
    let mut q = Quality { q: 2, last_ms: 0 };
    assert_eq!(q.update(low, 60, 10_000), 0);
    // Both flags set: start value 1.
    let both = LightOptions {
        low_quality: true,
        missile_lights: true,
    };
    let mut q = Quality::default();
    assert_eq!(q.update(both, 60, 10_000), 1);
    // Medium (0, 0): start value 2.
    let mut q = Quality::default();
    assert_eq!(
        q.update(LightOptions::from_menu(QualityOption::Medium), 16, 10_000),
        2
    );
}

#[test]
fn quality_13_to_15_early_return() {
    let hi = LightOptions::default();
    let mut q = Quality { q: 2, last_ms: 0 };
    assert_eq!(q.update(hi, 14, 50_000), 2);
    assert_eq!(q, Quality { q: 2, last_ms: 0 }, "no timer update");
    let mut q = Quality { q: 1, last_ms: 7 };
    assert_eq!(q.update(hi, 13, 50_000), 1);
    assert_eq!(q.last_ms, 7);
}

#[test]
fn quality_hysteresis() {
    let hi = LightOptions::default();
    let mut q = Quality {
        q: 2,
        last_ms: 1_000,
    };
    assert_eq!(q.update(hi, 5, 3_000), 2, "exactly 2,000 ms: kept");
    assert_eq!(q.last_ms, 1_000);
    assert_eq!(q.update(hi, 5, 3_001), 0);
    assert_eq!(q.last_ms, 3_001);
    // Same candidate: no change, no timer update.
    assert_eq!(q.update(hi, 5, 9_000), 0);
    assert_eq!(q.last_ms, 3_001);
}

#[test]
fn draw_rate_meter() {
    let mut m = DrawRateMeter::new(100);
    assert_eq!(m.d, 25);
    for _ in 0..47 {
        m.count_draw();
    }
    assert_eq!(m.pass(3_100), 25, "3,000 ms exactly: window kept");
    assert_eq!(m.count, 47);
    assert_eq!(m.pass(3_101), 15, "47 / 3");
    assert_eq!((m.count, m.window_start_ms), (0, 3_101));
}

// ---- the player's light radius ----

#[test]
fn tsv_matches_spec_tables() {
    let t = tables();
    let p = |start, kind, r, g, b| Period {
        start,
        kind,
        r,
        g,
        b,
    };
    assert_eq!(
        t.normal,
        [
            p(320, 3, 125, 144, 243),
            p(340, 3, 208, 184, 131),
            p(0, 0, 255, 255, 255),
            p(160, 1, 255, 255, 255),
            p(180, 1, 194, 152, 193),
            p(200, 2, 125, 144, 243),
        ]
    );
    assert_eq!(
        t.act4,
        [
            p(340, 3, 243, 70, 243),
            p(350, 3, 208, 184, 131),
            p(0, 0, 255, 20, 20),
            p(180, 1, 255, 255, 30),
            p(190, 1, 20, 152, 193),
            p(200, 2, 125, 144, 243),
        ]
    );
    assert_eq!(
        t.eclipse,
        [
            p(300, 3, 0, 30, 243),
            p(0, 0, 0, 30, 244),
            p(60, 1, 0, 30, 243),
            p(120, 2, 0, 30, 243),
            p(180, 2, 0, 30, 244),
            p(240, 2, 0, 30, 243),
        ]
    );
}

#[test]
fn tsv_strict() {
    let bad_header = ENV_PERIODS_TSV.replacen("start_deg", "start", 1);
    assert!(matches!(
        PeriodTables::parse(&bad_header),
        Err(TsvError::Header(_))
    ));
    let short: String = ENV_PERIODS_TSV
        .lines()
        .take(18)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(PeriodTables::parse(&short), Err(TsvError::RowCount(17)));
    let extra = format!("{ENV_PERIODS_TSV}eclipse\t5\t240\t2\t0\t30\t243\n");
    assert_eq!(PeriodTables::parse(&extra), Err(TsvError::RowCount(19)));
    for (from, to) in [
        ("normal\t1\t340", "normal\t2\t340"),
        ("act4\t0\t340", "normal\t0\t340"),
        ("\t125\t144\t243", "\t125\t144\t256"),
        ("normal\t0\t320\t3", "normal\t0\tx\t3"),
        ("normal\t0\t320\t3", "normal\t0\t360\t3"),
        ("\t208\t184\t131", "\t208\t184"),
    ] {
        let text = ENV_PERIODS_TSV.replacen(from, to, 1);
        assert_ne!(text, ENV_PERIODS_TSV);
        assert!(
            matches!(PeriodTables::parse(&text), Err(TsvError::Row { .. })),
            "{from:?} → {to:?}"
        );
    }
}

#[test]
fn creation() {
    let e = Environment::new(&tables(), 4321);
    assert_eq!((e.index, e.kind, e.ticks, e.intensity), (2, 0, 0, 128));
    assert_eq!((e.r, e.g, e.b), (255, 255, 255));
    assert_eq!((e.speed, e.eclipse, e.created_ms), (128, false, 4321));
}

#[test]
fn act_index_from_level() {
    for (l, a) in [
        (0, 0),
        (1, 0),
        (39, 0),
        (40, 1),
        (74, 1),
        (75, 2),
        (102, 2),
        (103, 3),
        (108, 3),
        (109, 4),
        (136, 4),
    ] {
        assert_eq!(act_index(l), a, "level {l}");
    }
}

#[test]
fn intensity_normal_act1_and_act5() {
    let t = tables();
    let degs = [0, 30, 45, 90, 135, 179, 180, 200, 270, 300, 339];
    let act1 = [128, 192, 219, 255, 219, 130, 128, 106, 64, 73, 105];
    let act5 = [128, 170, 170, 170, 170, 130, 128, 106, 64, 73, 105];
    for (k, &deg) in degs.iter().enumerate() {
        for (a, want) in [(0, act1[k]), (4, act5[k])] {
            let mut e = env_at(&t, 2, deg);
            e.intensity(a, 1);
            assert_eq!(e.intensity, want, "A {a}, degree {deg}");
        }
    }
}

#[test]
fn intensity_act4_eclipse_level120() {
    let t = tables();
    let mut e = env_at(&t, 2, 0);
    e.intensity = 100;
    e.intensity(3, 103);
    assert_eq!(e.intensity, 101);
    e.intensity = 100;
    e.intensity(3, 105);
    assert_eq!(e.intensity, 99);
    e.intensity = -1;
    e.intensity(3, 107);
    assert_eq!(e.intensity, 16, "−1 → 0 → target");
    e.intensity = 0;
    e.intensity(3, 104);
    assert_eq!(e.intensity, 1);
    e.eclipse = true;
    e.intensity = 100;
    e.intensity(0, 1);
    assert_eq!(e.intensity, 92);
    e.intensity = 36;
    e.intensity(0, 1);
    assert_eq!(e.intensity, 32);
    e.eclipse = false;
    e.intensity(0, 120);
    assert_eq!(e.intensity, 200);
}

#[test]
fn color_vectors() {
    let t = tables();
    for (index, deg, want) in [
        (3, 170, (225, 204, 225)),
        (4, 190, (160, 149, 218)),
        (0, 330, (167, 164, 188)),
    ] {
        let mut e = env_at(&t, index, deg);
        e.color(&t, 0);
        assert_eq!((e.r, e.g, e.b), want, "index {index}");
    }
}

#[test]
fn advance_steps() {
    let t = tables();
    // Day (type 0) +1; night (type 2) +2, +10 in A = 2; A = 3 +16; eclipse +1.
    for (index, a, eclipse, step) in [
        (2, 0, false, 1),
        (5, 0, false, 2),
        (5, 2, false, 10),
        (2, 3, false, 16),
        (5, 3, false, 16),
        (5, 2, true, 1),
    ] {
        let deg = if index == 2 { 100 } else { 210 };
        let mut e = env_at(&t, index, deg);
        e.eclipse = eclipse;
        let before = e.ticks;
        e.advance(&t, a);
        assert_eq!(
            e.ticks - before,
            step,
            "index {index}, A {a}, eclipse {eclipse}"
        );
    }
    // After a full day the clock wraps.
    let mut e = env_at(&t, 1, 0);
    e.eclipse = true;
    e.ticks = SPEED * 360 - 1;
    e.advance(&t, 0);
    assert_eq!(e.ticks, 0);
}

#[test]
fn period_one_lasts_one_update() {
    let t = tables();
    let mut e = env_at(&t, 0, 340);
    e.kind = 3;
    e.advance(&t, 0);
    assert_eq!((e.index, e.ticks), (1, 340 * SPEED), "enters period 1");
    e.advance(&t, 0);
    assert_eq!(
        (e.index, e.kind, e.ticks),
        (2, 0, 0),
        "next update: period 2"
    );
    // Period 5 → 0 at 320 degrees, with the type.
    let mut e = env_at(&t, 5, 320);
    e.advance(&t, 0);
    assert_eq!((e.index, e.kind, e.ticks), (0, 3, 320 * SPEED));
    // Exactly at the start (not <): stays.
    let mut e = env_at(&t, 2, 160);
    e.ticks -= 1;
    e.advance(&t, 0);
    assert_eq!((e.index, e.ticks), (2, 160 * SPEED));
}

#[test]
fn eclipse_advance_tables() {
    let t = tables();
    // Eclipse, A = 3: the act 4 table is tested (entry 3 = 180), the
    // eclipse table supplies the new values (entry 3 = 120).
    let mut e = env_at(&t, 2, 180);
    e.eclipse = true;
    e.advance(&t, 3);
    assert_eq!((e.index, e.kind, e.ticks), (3, 2, 120 * SPEED));
    // Eclipse, A = 0: the eclipse table (entry 3 starts at 120).
    let mut e = env_at(&t, 2, 120);
    e.eclipse = true;
    e.advance(&t, 0);
    assert_eq!(e.index, 3);
}

#[test]
fn update_and_level_120() {
    let t = tables();
    let mut e = Environment::new(&t, 0);
    e.update(&t, 1);
    assert_eq!((e.index, e.ticks, e.intensity), (2, 1, 128));
    e.update(&t, 120);
    assert_eq!(e.intensity, 200);
    assert_eq!((e.r, e.g, e.b), (245, 240, 255));
}

#[test]
fn server_setter() {
    let t = tables();
    let mut e = Environment::new(&t, 0);
    assert_eq!(
        e.set_from_server(&t, 6, 0, 0, 0, 1),
        Err(EnvError::BadIndex(6))
    );
    assert_eq!(
        e.set_from_server(&t, -1, 0, 0, 0, 1),
        Err(EnvError::BadIndex(-1))
    );
    assert_eq!(
        e.set_from_server(&t, 3, -5, 0, 0, 1),
        Err(EnvError::NegativeTicks(-5))
    );

    e.set_from_server(&t, 3, SPEED * 360 + 1, 0, 0, 1).unwrap();
    assert_eq!((e.index, e.kind, e.ticks, e.intensity), (3, 1, 0, 128));
    e.set_from_server(&t, 4, SPEED * 360, 0, 0, 1).unwrap();
    assert_eq!(e.ticks, SPEED * 360, "= speed × 360 is kept");
    e.set_from_server(&t, 2, 90 * SPEED, 0, 0, 1).unwrap();
    assert_eq!(e.intensity, 255);
    assert_eq!((e.r, e.g, e.b), (255, 255, 255), "no color step");
    e.set_from_server(&t, 2, 90 * SPEED, 0, 0, 120).unwrap();
    assert_eq!((e.intensity, e.r, e.g, e.b), (200, 245, 240, 255));
}

#[test]
fn server_setter_eclipse_resets_the_period() {
    // The eclipse message: index 5, ticks 0, eclipse 1, previous flag 0, act 2
    // (`A` = 1, `L` = 40).
    let t = tables();
    let mut e = Environment::new(&t, 0);
    e.set_from_server(&t, 2, 0, 0, 1, 40).unwrap();
    assert_eq!(e.intensity, 128, "θ = 0 without the flag");
    e.set_from_server(&t, 5, 0, 1, 1, 40).unwrap();
    assert!(e.eclipse);
    // Period reset: eclipse entry 5 → type 2, ticks 240 × speed, the
    // received ticks discarded.
    assert_eq!((e.index, e.kind, e.ticks), (5, 2, 240 * SPEED));
    assert_eq!(240 * SPEED, 30_720);
    // First intensity with the previous flag (θ = 0: 128), then with the
    // eclipse: 128 − 8.
    assert_eq!(e.intensity, 120);
    assert_eq!((e.r, e.g, e.b), (0, 30, 243), "eclipse entry 5");
}

#[test]
fn eclipse_triggers() {
    assert_eq!(ECLIPSE_SET, (5, 0, 1));
    let mut tr = EclipseTrigger::default();
    assert!(!tr.on_quest_message(9, 1, true));
    assert!(!tr.on_quest_message(10, 2, true));
    assert!(tr.on_quest_message(10, 1, true));
    assert!(!tr.pending);
    assert!(!tr.on_act_load(1));
    assert!(!tr.on_quest_message(10, 3, false));
    assert!(tr.pending);
    assert!(!tr.on_act_load(0));
    assert!(tr.on_act_load(1));
}

#[test]
fn room_ambient_order() {
    let t = tables();
    let env = Environment::new(&t, 0);
    let a = |i, r, g, b| Ambient { i, r, g, b };
    let leveldefs = a(140, 255, 255, 255);
    assert_eq!(
        room_ambient(a(80, 255, 64, 48), leveldefs, &env),
        a(80, 255, 64, 48)
    );
    assert_eq!(room_ambient(a(80, 0, 0, 0), leveldefs, &env), leveldefs);
    assert_eq!(
        room_ambient(Ambient::ZERO, a(0, 0, 0, 1), &env),
        a(0, 0, 0, 1)
    );
    assert_eq!(
        room_ambient(Ambient::ZERO, a(140, 0, 0, 0), &env),
        a(128, 255, 255, 255),
        "environment"
    );
}

// ---- the level overrides ----

#[test]
fn sine_table_w7() {
    let w = sine_table();
    assert_eq!(w[7], 0.08579731732606888_f64 as f32);
    assert_eq!(w[128], 1.0);
    assert_eq!(w[0], 0.0);
}

#[test]
fn den_counter_intensities() {
    let mut o = Overrides::default();
    assert_eq!(o.ambient(8, 0), Ambient::ZERO, "quest byte 0");
    let glow = |i| Ambient {
        i,
        r: 255,
        g: 64,
        b: 48,
    };
    assert_eq!(o.ambient(8, 1), glow(80), "counter −1");
    for (c, i) in [(0, 80), (15, 56), (29, 4)] {
        o.den_counter = c;
        assert_eq!(o.ambient(8, 1), glow(i), "counter {c}");
    }
    // Past 29: flag set, override off.
    assert!(o.update_counters());
    assert_eq!(o.den_counter, 30);
    assert!(o.den_flag);
    assert_eq!(o.ambient(8, 1), Ambient::ZERO);
    assert!(!o.update_counters());
    assert_eq!(o.den_counter, 30, "no rise once the flag is set");
    assert!(o.room_load_gets_den_lights(8));
    assert!(!o.room_load_gets_den_lights(9));
    o.game_start();
    assert_eq!((o.den_flag, o.den_counter), (false, -1));
}

#[test]
fn den_light_placement() {
    // Every point free: 3 placed after 3 tries (6 steps).
    let start = Seed::new(12345, 666);
    let mut seed = start;
    let pts = den_light_points((100, 200, 8, 8), &mut seed, |_, _| 0);
    let mut s = start;
    let want: Vec<(i32, i32)> = (0..3)
        .map(|_| {
            let x = 100 + (s.step() % 8) as i32;
            let y = 200 + (s.step() % 8) as i32;
            (x, y)
        })
        .collect();
    assert_eq!(pts, want);
    assert_eq!(seed, s);
    // Every point blocked: 25 tries, 50 steps, nothing placed.
    let mut seed = start;
    assert!(den_light_points((0, 0, 5, 7), &mut seed, |_, _| 5).is_empty());
    let mut s = start;
    for _ in 0..50 {
        s.step();
    }
    assert_eq!(seed, s);
}

#[test]
fn red_glow_levels() {
    let mut o = Overrides::default();
    assert_eq!(o.ambient(107, 0), Ambient::ZERO);
    o.glow_flag = true;
    let glow = Ambient {
        i: 160,
        r: 255,
        g: 64,
        b: 48,
    };
    assert_eq!(o.ambient(107, 0), glow);
    assert_eq!(o.ambient(108, 0), glow);
    o.glow_counter = 0;
    assert_eq!(o.ambient(108, 0), Ambient::ZERO);
    for _ in 0..29 {
        o.update_counters();
    }
    assert_eq!((o.glow_flag, o.glow_counter), (true, 29));
    o.update_counters();
    assert_eq!((o.glow_flag, o.glow_counter), (false, -1));
}

#[test]
fn darkness_event() {
    let mut o = Overrides::default();
    assert_eq!(o.ambient(2, 0), Ambient::ZERO, "no event");
    o.start_darkness(4, 2, 0, 2);
    assert_eq!(o.darkness.unwrap().fade_out, 25);
    let base = Ambient {
        i: 200,
        r: 1,
        g: 2,
        b: 3,
    };
    let w = sine_table();
    let mut seen = Vec::new();
    for _ in 0..31 {
        o.update_darkness(base, 2);
        seen.push(o.ambient(2, 0).i);
    }
    let lvl = |a: usize| (f64::from(w[(a + 128) & 511]) * 200.0) as i32 as u8;
    // In: a lags one update (0, 0, 32, 64); hold: 0; out: from a = 96.
    assert_eq!(&seen[..4], &[lvl(0), lvl(0), lvl(32), lvl(64)]);
    assert_eq!(&seen[4..6], &[0, 0]);
    assert_eq!(seen[6], lvl(96), "a from the last in step");
    assert_eq!(seen[7], lvl(128));
    let e = o.darkness.unwrap();
    assert_eq!((e.c, e.out.r, e.out.g, e.out.b), (31, 1, 2, 3));
    // c > total (31) clears it.
    o.update_darkness(base, 2);
    assert_eq!(o.darkness, None);
    assert_eq!(o.ambient(2, 0), Ambient::ZERO);
    // Another level clears it after one step.
    o.start_darkness(4, 2, 5, 2);
    o.update_darkness(base, 3);
    assert_eq!(o.darkness, None);
}

#[test]
fn unique_event_dispatch() {
    let mut o = Overrides::default();
    let never = || panic!("darkness callback only for unique event id 13");
    assert_eq!(
        o.unique_event(32, 1, never),
        Err(OverrideError::Fatal1D9(32))
    );
    assert_eq!(
        o.unique_event(255, 1, never),
        Err(OverrideError::Fatal1D9(255))
    );
    assert_eq!(
        o.unique_event(20, 1, never),
        Err(OverrideError::Fatal1DA(20))
    );
    assert_eq!(o.unique_bits, 0);
    assert_eq!(
        o.unique_event(0, 1, never),
        Ok(UniqueEventEffect::DenCounterStarted)
    );
    assert_eq!(o.den_counter, 0);
    assert_eq!(
        o.unique_event(12, 108, never),
        Ok(UniqueEventEffect::RedGlowStarted { in_level_108: true })
    );
    assert!(o.glow_flag);
    assert_eq!(
        o.unique_event(13, 107, || 7),
        Ok(UniqueEventEffect::RedGlowCounterStarted)
    );
    assert_eq!((o.glow_counter, o.glow_value), (0, 97));
    for id in [1, 3, 6, 14, 16, 17, 19] {
        assert_eq!(
            o.unique_event(id, 1, never),
            Ok(UniqueEventEffect::OtherClientEffect(id))
        );
    }
    assert_eq!(o.unique_event(2, 1, never), Ok(UniqueEventEffect::None));
    assert_eq!(
        o.unique_bits,
        (1 << 0)
            | (1 << 1)
            | (1 << 2)
            | (1 << 3)
            | (1 << 6)
            | (1 << 12)
            | (1 << 13)
            | (1 << 14)
            | (1 << 16)
            | (1 << 17)
            | (1 << 19)
    );
}

// ---- the ambient colour ----

fn req(kind: u8, radius: i32, rgb: (u8, u8, u8), target: Option<i32>) -> LightRequest {
    LightRequest {
        kind,
        radius,
        i: 255,
        r: rgb.0,
        g: rgb.1,
        b: rgb.2,
        target,
    }
}

#[test]
fn player_light() {
    assert_eq!(
        sources::player_light(true),
        req(0, 13, (255, 255, 255), None)
    );
    assert_eq!(
        sources::player_light(false),
        req(1, 13, (255, 255, 255), None)
    );
    assert_eq!(sources::player_light_radius(4), 17);
    assert_eq!(sources::player_light_color(0), (255, 255, 255));
    assert_eq!(sources::player_light_color(0x00AA_BBCC), (0xAA, 0xBB, 0xCC));
    assert_eq!(sources::player_light_color(0xFF00_0001), (0, 0, 1));
}

#[test]
fn monster_light() {
    assert_eq!(sources::component_light([]), 0);
    assert_eq!(sources::component_light([3, 9, 2]), 9);
    let base = MonsterLightInput {
        l_c: 0,
        light: 5,
        rgb: (230, 168, 255),
        level: 2,
        quest_byte1: true,
        client_only: false,
        align: 0,
    };
    assert_eq!(
        sources::monster_light(&base),
        Some(req(0, 5, (230, 168, 255), None))
    );
    let lc = MonsterLightInput { l_c: 9, ..base };
    assert_eq!(sources::monster_light(&lc).unwrap().radius, 9);
    let none = MonsterLightInput { light: 0, ..base };
    assert_eq!(sources::monster_light(&none), None);
    let den = MonsterLightInput { level: 8, ..base };
    assert_eq!(sources::monster_light(&den).unwrap().radius, 3);
    let den0 = MonsterLightInput {
        level: 8,
        light: 0,
        ..base
    };
    assert_eq!(sources::monster_light(&den0).unwrap().radius, 3);
    for m in [
        MonsterLightInput {
            quest_byte1: false,
            ..den
        },
        MonsterLightInput {
            client_only: true,
            ..den
        },
        MonsterLightInput { align: 1, ..den },
        MonsterLightInput { align: 2, ..den },
    ] {
        assert_eq!(sources::monster_light(&m).unwrap().radius, 5);
    }
}

#[test]
fn umod3_light() {
    let start = Seed::new(777, 666);
    let mut seed = start;
    assert_eq!(sources::umod3_light(0, true, &mut seed, (1, 2, 3)), None);
    assert_eq!(seed, start, "u = 0: no step");
    assert_eq!(
        sources::umod3_light(8, false, &mut seed, (1, 2, 3)),
        Some(req(0, 7, (1, 2, 3), None))
    );
    assert_eq!(seed, start);
    let mut s = start;
    let (w1, w2, w3) = (s.step(), s.step(), s.step());
    assert_eq!(
        sources::umod3_light(8, true, &mut seed, (1, 2, 3)),
        Some(req(0, 7, (w3 as u8, w2 as u8, w1 as u8), None))
    );
    assert_eq!(seed, s);
}

#[test]
fn overlay_object_horadric_cursecenter() {
    let c = (10, 20, 30);
    assert_eq!(sources::overlay_light(4, 0, c), None);
    assert_eq!(sources::overlay_light(4, 4, c), Some(req(1, 4, c, None)));
    assert_eq!(sources::overlay_light(1, 6, c), Some(req(1, 1, c, Some(6))));
    assert_eq!(sources::object_light(0, true, c), ObjectLight::Remove);
    assert_eq!(
        sources::object_light(9, false, c),
        ObjectLight::Create(req(2, 4, c, None))
    );
    assert_eq!(
        sources::object_light(12, true, c),
        ObjectLight::SetTarget(6)
    );
    assert_eq!(sources::horadric_light(), req(2, 60, (255, 255, 255), None));
    assert_eq!(
        sources::cursecenter_light(0),
        req(1, 1, (255, 0, 0), Some(2))
    );
    assert_eq!(
        sources::cursecenter_light(7),
        req(1, 1, (255, 0, 0), Some(7))
    );
}

#[test]
fn skill_cast_light() {
    let c = (1, 2, 3);
    assert_eq!(sources::skill_cast_light(0, c), None);
    assert_eq!(sources::skill_cast_light(8, c), Some(req(1, 1, c, Some(8))));
}

#[test]
fn missile_light() {
    let c = (1, 2, 3);
    assert_eq!(
        sources::missile_light(true, 0, 8, 0, c),
        Some(req(1, 8, c, None))
    );
    assert_eq!(
        sources::missile_light(false, 0, 8, 0, c),
        None,
        "not high quality"
    );
    assert_eq!(sources::missile_light(true, 0x4000, 8, 0, c), None);
    assert_eq!(sources::missile_light(true, 0, 0, 0, c), None);
    assert_eq!(
        sources::missile_light(true, 0, 0, 1, c),
        Some(req(1, 0, c, None)),
        "+0x50: a radius-0 create (nothing created)"
    );
}

#[test]
fn missile_flicker() {
    let start = Seed::new(4242, 666);
    let mut seed = start;
    for (frame, radius, flicker) in [
        (0x100, Some(10), 3),
        (0, None, 3),
        (0, Some(7), 3),
        (0, Some(10), 0),
    ] {
        assert_eq!(
            sources::missile_flicker(frame, radius, 8, flicker, &mut seed),
            None
        );
    }
    assert_eq!(seed, start, "no step");
    let mut s = start;
    let want = 8 + (s.step() % 3) as i32;
    assert_eq!(
        sources::missile_flicker(0x400 | 0xFF, Some(8), 8, 3, &mut seed),
        Some(want)
    );
    assert_eq!(seed, s);
}

#[test]
fn overrides_are_keyed_by_the_room_level() {
    // One override state; only the level id picks the rule: the Den (8)
    // glows, the levels 107 / 108 rule needs its own flag, any other level
    // falls through (R, G, B 0) while no darkness event runs.
    let o = Overrides::default();
    assert!(o.ambient(8, 1).has_color());
    for level in [1, 7, 9, 107, 108, 120] {
        assert_eq!(o.ambient(level, 1), Ambient::ZERO, "level {level}");
    }
    let glow = Overrides {
        glow_flag: true,
        ..Overrides::default()
    };
    assert!(glow.ambient(107, 0).has_color() && glow.ambient(108, 0).has_color());
    assert_eq!(glow.ambient(9, 0), Ambient::ZERO);
}
