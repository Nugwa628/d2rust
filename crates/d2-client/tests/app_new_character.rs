//! `play --new <class> <name>` (decision D3): the join loads a new
//! character of the request's class and name on the synthetic game,
//! held in memory only.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use d2_client::app::single_player::{self, DEFAULT_SEED};
use d2_client::bridge::link::{SendQueue, ServerLink};

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl d2_server::seams::Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The stub load (0x5F after 0x0B) of a
/// `--new` barbarian named "Conan".
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_join_creates_the_named_character() {
    let ms = Arc::new(AtomicU32::new(1000));
    let character = single_player::new_character("barbarian", "Conan").unwrap();
    let (mut link, _) = single_player::start_with(
        app_support::game_data(),
        DEFAULT_SEED,
        character.clone(),
        StepClock(ms.clone()),
    )
    .unwrap();
    let req = single_player::create_request_for(&character);
    link.send(SendQueue::System, &req.encode()).unwrap();
    link.pump().unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    link.receive();
    link.send(SendQueue::System, &[0x6B]).unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    assert!(link.pump().unwrap().ticked);
    let ids: Vec<u8> = link.receive().iter().map(|c| c[0]).collect();
    assert!(ids.windows(2).any(|w| w == [0x0B, 0x5F]), "{ids:02X?}");
    let (class, name) = link
        .with(|l| {
            let sim = &l.host().game;
            let (p, _) = single_player::local_player(sim).expect("joined");
            (
                sim.events.action.sys.units.get(p).map(|u| u.class),
                sim.world.rest.names.get(&p).cloned(),
            )
        })
        .unwrap();
    assert_eq!(class, Some(4));
    assert_eq!(name.as_deref(), Some(&b"Conan"[..]));
}

/// `play --difficulty hell`: the 0x67 asks for Hell, the game runs on it
/// (S→C 0x01 byte 1, the sim's difficulty copy), and the new
/// character's save town is Hell's.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_join_runs_the_game_on_the_chosen_difficulty() {
    let ms = Arc::new(AtomicU32::new(1000));
    let character = single_player::new_character("amazon", "Hel")
        .unwrap()
        .with_difficulty(2);
    assert_eq!(character.difficulty(), 2);
    let (mut link, _) = single_player::start_with(
        app_support::game_data(),
        DEFAULT_SEED,
        character.clone(),
        StepClock(ms.clone()),
    )
    .unwrap();
    let req = single_player::create_request_for(&character);
    assert_eq!(req.difficulty, 2);
    link.send(SendQueue::System, &req.encode()).unwrap();
    link.pump().unwrap();
    ms.fetch_add(40, Ordering::SeqCst);
    link.pump().unwrap();
    let got = link.receive();
    let game = got.iter().find(|c| c[0] == 0x01).expect("S→C 0x01");
    assert_eq!(game[1], 2);
    let d = link
        .with(|l| l.host_mut().game.events.action.hooks().ai_info.difficulty)
        .unwrap();
    assert_eq!(d, 2);
    let save = d2_client::app::save::base_save(&character);
    assert_eq!(save.header.towns, [0, 0, 0x80]);
}

#[test]
fn difficulty_names_parse() {
    assert_eq!(single_player::parse_difficulty("Nightmare"), Some(1));
    assert_eq!(single_player::parse_difficulty("hell"), Some(2));
    assert_eq!(single_player::parse_difficulty("3"), None);
    assert_eq!(single_player::Character::New.difficulty(), 0);
}
