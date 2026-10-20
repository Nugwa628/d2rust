//! The animation rate of a cast and the shape it leaves, on the user's
//! install, against the game (reference runs of a werewolf Druid and of
//! Burst of Speed, 2026-10-09, in an empty Blood Moor): the server's rate
//! is the unit rate formula on the draw identity (the Werewolf's state
//! 139 draws the Druid as monster 430), and a state's stat fill re-rates
//! the running mode (the animation refresh).

use d2_client::app::single_player::Character;
use d2_client::app::state_dump::{self, DumpArgs, DumpGame, RunInfo};
use d2_formats::d2s::{self, Body, D2s, Golem, Header, StatEntry, Stats};
use d2_server::adapters::character::LoadContext;

use crate::app_support;

/// `d2s-tool new --class C --expansion --level 30 --all-skills 20
/// --right-skill R`: life, mana and stamina as the tool writes them.
fn save(name: &[u8], class: u8, stats: [(u16, i32); 10], right: i32) -> Character {
    let mut header = Header::default();
    header.set_name(name).unwrap();
    header.class = class;
    header.level = 30;
    header.status = d2s::status::EXPANSION;
    header.towns = [0x80, 0, 0];
    header.mouse[1] = d2s::Slot::encode(right, false, 0).unwrap();
    let mut stats: Vec<StatEntry> = stats
        .into_iter()
        .map(|(id, value)| StatEntry {
            id,
            layer: 0,
            value,
        })
        .collect();
    // Stat points, skill points, level and its experience.
    for (id, value) in [(4, 145), (5, 29), (12, 30), (13, 4_663_553)] {
        stats.push(StatEntry {
            id,
            layer: 0,
            value,
        });
    }
    stats.sort_by_key(|e| e.id);
    let save = D2s {
        header,
        body: Some(Body {
            stats: Stats::Bits(stats),
            skills: vec![20; 30],
            hireling_items: Some(None),
            golem: Some(Golem {
                flag: 0,
                item: None,
            }),
            ..Body::default()
        }),
    };
    Character::Save(
        Box::new(save),
        LoadContext {
            difficulty: 0,
            map_seed_applies: false,
        },
    )
}

/// The player's (mode, frame count, speed) by frame, after `warp 2`
/// before frame 4 and a right click at (330, 300) before frame 20.
fn player_rates(character: Character, ticks: u32) -> Vec<(u32, String)> {
    let args = DumpArgs {
        tick_rate: d2_sim::tick::TickRate::BASE,
        save: None,
        seed: Some(1234),
        difficulty: 0,
        ticks,
        every: 1,
        out: "unused".into(),
        game_dir: None,
        date: Some("2026-10-09".into()),
        pokes: vec![d2_client::app::poke::parse_poke_arg("4 warp 2").unwrap()],
        input: Some(
            d2_client::world_view::input_script::parse("frame 20; rclick 330 300").unwrap(),
        ),
        packets: None,
        rng: None,
        save_out: None,
        client_out: None,
        client_grid: false,
        early: Vec::new(),
        no_ping: false,
        sends: Vec::new(),
        no_own_c2s: Vec::new(),
        send_clock: None,
        player_seed: None,
        gfx_ready: None,
    };
    let mut game = DumpGame::resolve(&args, app_support::game_data(), None).unwrap();
    game.character = character;
    let info = RunInfo {
        tool: "d2-client state-dump test".into(),
        date: "2026-10-09".into(),
        command: "test".into(),
        save: Some("Scn.d2s".into()),
    };
    let mut out = Vec::new();
    state_dump::dump(game, ticks, 1, &info, &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .filter(|l| l.starts_with(r#"{"k":"snap""#) && l.contains(r#"{"ut":0,"#))
        .map(|l| {
            let f: u32 = l[r#"{"k":"snap","f":"#.len()..]
                .split(',')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            let p = &l[l.find(r#"{"ut":0,"#).unwrap()..];
            let field = |k: &str| {
                let at = p.find(&format!(r#""{k}":"#)).unwrap() + k.len() + 3;
                p[at..].split([',', '}']).next().unwrap().to_string()
            };
            (
                f,
                format!("m={} fc={} sp={}", field("m"), field("fc"), field("sp")),
            )
        })
        .collect()
}

fn at(rates: &[(u32, String)], f: u32) -> &str {
    &rates.iter().find(|(g, _)| *g == f).unwrap().1
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn werewolf_re_rates_the_cast_and_draws_the_druid_as_the_wolf() {
    let stats = [
        (0, 15),
        (1, 20),
        (2, 20),
        (3, 25),
        (6, 25216),
        (7, 25216),
        (8, 19968),
        (9, 19968),
        (10, 28928),
        (11, 28928),
    ];
    let r = player_rates(save(b"ScnDru", 5, stats, 223), 38);
    // The game: the cast (SC, 15 frames) at the Druid's 208; the shape lands
    // at frame 29 and re-rates the running cast to 168; the wolf's
    // neutral: 9 frames at 241.
    assert_eq!(at(&r, 28), "m=10 fc=3840 sp=208");
    assert_eq!(at(&r, 29), "m=10 fc=3840 sp=168");
    assert_eq!(at(&r, 38), "m=1 fc=2304 sp=241");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn burst_of_speed_raises_the_neutral_rate() {
    let stats = [
        (0, 20),
        (1, 25),
        (2, 20),
        (3, 20),
        (6, 27648),
        (7, 27648),
        (8, 17536),
        (9, 17536),
        (10, 33600),
        (11, 33600),
    ];
    let r = player_rates(save(b"ScnAss", 6, stats, 258), 36);
    // The game: the cast at 256; the neutral after it at 194.
    assert_eq!(at(&r, 35), "m=10 fc=4352 sp=256");
    assert_eq!(at(&r, 36), "m=1 fc=2048 sp=194");
}
