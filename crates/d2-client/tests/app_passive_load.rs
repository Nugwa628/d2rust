//! A save's passive skills count from the join, on the user's install,
//! against what the game does with the same save (a reference run,
//! 2026-10-09): the load's skill section assigns each skill, which turns
//! its passive state on and
//! fills the state's stat list: an expansion Barbarian
//! with every skill at 20 has Increased Stamina's +315 % in his maximum
//! stamina at frame 2.

use d2_client::app::single_player::Character;
use d2_client::app::state_dump::{self, DumpArgs, DumpGame, RunInfo};
use d2_formats::d2s::{self, Body, D2s, Golem, Header, StatEntry, Stats};
use d2_server::adapters::character::LoadContext;

use crate::app_support;

/// ScnBar of the check: expansion Barbarian, level 30, every skill at 20.
fn scn_bar() -> Character {
    let mut header = Header::default();
    header.set_name(b"ScnBar").unwrap();
    header.class = 4;
    header.level = 30;
    header.status = d2s::status::EXPANSION;
    header.towns = [0x80, 0, 0];
    // As `d2s-tool new --level 30` writes them: life 113, mana 39,
    // stamina 121 (1/256 points).
    let stats = [
        (0, 30),
        (1, 10),
        (2, 20),
        (3, 25),
        (6, 28928),
        (7, 28928),
        (8, 9984),
        (9, 9984),
        (10, 30976),
        (11, 30976),
        (12, 30),
    ]
    .map(|(id, value)| StatEntry {
        id,
        layer: 0,
        value,
    })
    .to_vec();
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

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_loaded_increased_stamina_raises_the_maximum_stamina_from_the_join() {
    let args = DumpArgs {
        tick_rate: d2_sim::tick::TickRate::BASE,
        save: None,
        seed: Some(1234),
        difficulty: 0,
        ticks: 2,
        every: 1,
        out: "unused".into(),
        game_dir: None,
        date: Some("2026-10-09".into()),
        pokes: Vec::new(),
        sends: Vec::new(),
        no_own_c2s: Vec::new(),
        send_clock: None,
        player_seed: None,
        gfx_ready: None,
        input: None,
        packets: None,
        rng: None,
        save_out: None,
        client_out: None,
        client_grid: false,
        early: Vec::new(),
        no_ping: false,
    };
    let mut game = DumpGame::resolve(&args, app_support::game_data(), None).unwrap();
    game.character = scn_bar();
    let info = RunInfo {
        tool: "d2-client state-dump test".into(),
        date: "2026-10-09".into(),
        command: "test".into(),
        save: Some("ScnBar.d2s".into()),
    };
    let mut out = Vec::new();
    state_dump::dump(game, 2, 1, &info, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let snap = text
        .lines()
        .find(|l| l.starts_with(r#"{"k":"snap","f":2,"#))
        .expect("frame 2");
    // The game: base 121 stamina (30976) · (100 + 315) / 100 = 128550.
    assert!(
        snap.contains(r#""st":128550,"stx":128550,"#),
        "{}",
        &snap[..snap.len().min(600)]
    );
}
