//! Local-link and presentation adapters over the headless server game.
use super::server_thread::ThreadLink;
use crate::bridge::local::{LocalLink, PendingSession};
use crate::bridge::LOCAL_CLIENT;
use d2_formats::d2s::D2s;
use d2_proto::envelope::Wire;
use d2_server::adapters::ProtoSizes;
use d2_server::host::{Host, SystemClock};
use d2_server::seams::Clock;
use d2_sim::rules::Ruleset;
use d2_sim::units::UnitId;
use std::sync::Arc;
mod builder;
mod client_tables;
pub use builder::*;
pub use client_tables::*;
pub use d2_server::game::rows;
pub use d2_server::game::{
    clock_map_seed, drawn_map_seed, game_seed, game_seed_or, load_character, load_character_with,
    loader, new_character, one_character, parse_class, parse_difficulty, refused_load, save_name,
    wire_view_gap, BuildError, Character, CharacterCodec, Characters, D2sCodec, GameData,
    GameSource, Layers, LiveData, LiveDs1, LiveSubs, NewCharacter, NewCharacterError, RefusedLoad,
    Sim, WaypointTables, World, ACT1_TOWN, ACT2_TOWN, ACT5_TOWN, BLOOD_MOOR, CATACOMBS_4,
    CLASS_NAMES, COLD_PLAINS, CREATE_FLAGS_CLASSIC, CREATE_FLAGS_EXPANSION, DEFAULT_SEED,
    DEN_OF_EVIL, DURANCE_1, DURANCE_2, DURIELS_LAIR, DURIEL_CLASS, GAME_SETUP, GAME_TYPE,
    KURAST_DOCKS, LOAD_FAILED, MAX_NAME_LEN, PANDEMONIUM_FORTRESS, PLAYER_CLASS, PLAYER_NAME,
    STASH_CLASS, STONY_FIELD,
};
pub use d2_server::local_seams::*;
pub type Link<C = SystemClock> = LocalLink<Sim, ProtoSizes, PendingSession, C>;
/// The local join request with the selected presentation width.
pub fn create_request_for(character: &Character) -> d2_server::adapters::session_flow::CreateGame {
    let frame = crate::rules::camera::FrameSize::play();
    d2_server::game::create_request_for(character, frame.width, frame.height)
}
/// The default character's local join request.
pub fn create_request() -> d2_server::adapters::session_flow::CreateGame {
    create_request_for(&Character::New)
}
/// The built simulation with the client's save destination.
pub struct LocalGame {
    pub sim: Sim,
    pub saves: Saves,
}
/// Build with the app's character defaults.
pub fn build(data: &GameData, seed: u32) -> Result<LocalGame, BuildError> {
    build_with(data, seed, Character::New)
}
/// Build with the selected character.
pub fn build_with(
    data: &GameData,
    seed: u32,
    character: Character,
) -> Result<LocalGame, BuildError> {
    build_with_rules(data, seed, character, Ruleset::Original)
}
/// Build with the selected rules and app save destination.
pub fn build_with_rules(
    data: &GameData,
    seed: u32,
    character: Character,
    rules: Ruleset,
) -> Result<LocalGame, BuildError> {
    build_game(data, seed, character, rules)
}
fn build_game(
    data: &GameData,
    seed: u32,
    character: Character,
    rules: Ruleset,
) -> Result<LocalGame, BuildError> {
    let GameData::Live(d) = data;
    let appearance = crate::world_view::unit_assets::UnitLooks::live(d.archives.as_ref())
        .ok()
        .map(|looks| Arc::new(looks) as Arc<dyn d2_server::local_seams::UnitAppearance>);
    Ok(LocalGame {
        sim: d2_server::game::GameBuilder::new(data.clone())
            .appearance(appearance)
            .seed(seed)
            .character(character)
            .rules(rules)
            .build()?,
        saves: Saves::d2rust(),
    })
}
/// The local client's player and its GUID once the join has run (C→S
/// 0x6B drained), else `None`.
pub fn local_player(s: &Sim) -> Option<(UnitId, u32)> {
    let p = s.player_of(LOCAL_CLIENT)?;
    Some((p, s.game.lists.unit(p)?.guid))
}

/// The units of a started game, for the caller (the player exists after
/// the join: [`local_player`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    /// The store prices the host publishes for the shop panel.
    pub prices: crate::ui::original::ShopPrices,
    /// The wire the host speaks: [`Wire::Mod`] for a mod game's rules
    /// (MS13). The client's bridge speaks the same
    /// (`Bridge::set_mod_messages`).
    pub wire: Wire,
}

/// Builds the game on a new server thread behind a local host with
/// `clock` (`SystemClock` in the app; a manual clock in tests).
pub fn start<C: Clock + Send + 'static>(
    data: GameData,
    seed: u32,
    clock: C,
) -> Result<(ThreadLink<Link<C>>, Started), BuildError> {
    start_with(data, seed, Character::New, clock)
}

/// [`start`] with the character the join loads.
pub fn start_with<C: Clock + Send + 'static>(
    data: GameData,
    seed: u32,
    character: Character,
    clock: C,
) -> Result<(ThreadLink<Link<C>>, Started), BuildError> {
    GameBuilder::new(data)
        .seed(seed)
        .character(character)
        .start(clock)
}

#[cfg(test)]
mod new_character_tests {
    use super::*;
    use d2_server::adapters::character::LoadContext;

    #[test]
    fn a_drawn_map_seed_is_two_time_values_around_one_step() {
        // time_value(0) = 0x51ccf6e9 seeds {0x51ccf6e9, 666}; one step,
        // then time_value of the clock plus the new low word.
        assert_eq!(d2_sim::rng::time_value(0), 0x51CC_F6E9);
        assert_eq!(drawn_map_seed(0, 0), 0x7C4D_925C);
        assert_eq!(drawn_map_seed(1_700_000_000, 12345), 0x166E_E911);
        assert!(clock_map_seed() <= 0x7FFF_FFFF);
    }

    #[test]
    fn the_map_seed_comes_from_the_switch_then_the_save() {
        let saved = |town: u8, difficulty: u8| {
            let mut s = d2_formats::d2s::D2s::new_stub(b"Seed", 1, 0x20, 0).unwrap();
            s.header.map_seed = 0x2468_ACE0;
            s.header.towns[usize::from(difficulty)] = town;
            Character::Save(
                Box::new(s),
                LoadContext {
                    difficulty,
                    map_seed_applies: false,
                },
            )
        };
        // The town byte's 0x80 for the game's difficulty restores it.
        assert_eq!(game_seed(&saved(0x80, 0), None), 0x2468_ACE0);
        assert_eq!(game_seed(&saved(0x82, 2), None), 0x2468_ACE0);
        // Without 0x80 (or on another difficulty): not restored.
        assert_eq!(game_seed(&saved(0x00, 0), None), DEFAULT_SEED);
        // `--seed N` (game +0x84 = 1) wins.
        assert_eq!(game_seed(&saved(0x80, 0), Some(7)), 7);
        assert_eq!(game_seed(&Character::New, None), DEFAULT_SEED);
        assert_eq!(game_seed(&Character::New, Some(9)), 9);
        // The front end's drawn seed replaces only the default.
        assert_eq!(game_seed_or(&Character::New, None, 77), 77);
        assert_eq!(game_seed_or(&Character::New, Some(9), 77), 9);
        assert_eq!(game_seed_or(&saved(0x80, 0), None, 77), 0x2468_ACE0);
        assert_eq!(game_seed_or(&saved(0x00, 0), None, 77), 77);
    }

    #[test]
    fn class_by_name_or_number() {
        for (i, name) in CLASS_NAMES.iter().enumerate() {
            assert_eq!(parse_class(name), Ok(i as u8));
            assert_eq!(parse_class(&name.to_uppercase()), Ok(i as u8));
            assert_eq!(parse_class(&i.to_string()), Ok(i as u8));
        }
        for bad in ["7", "255", "-1", "", "paladins", "ama"] {
            assert!(parse_class(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn names_fit_the_create_request() {
        let Ok(Character::Named(c)) = new_character("amazon", "Test") else {
            panic!("refused");
        };
        assert_eq!((c.class, c.name()), (0, &b"Test"[..]));
        assert_eq!(c.name[4..], [0; 12]);
        assert!(new_character("druid", &"A".repeat(15)).is_ok());
        for bad in ["", &"A".repeat(16), "two words", "Näme", "a\0b"] {
            assert!(
                matches!(
                    new_character("druid", bad),
                    Err(NewCharacterError::Name(..))
                ),
                "{bad:?}"
            );
        }
        assert!(matches!(
            new_character("monk", "Test"),
            Err(NewCharacterError::Class(_))
        ));
    }

    #[test]
    fn the_create_request_carries_the_new_class_and_name() {
        let c = new_character("6", "Shadow_1").unwrap();
        let r = create_request_for(&c);
        assert_eq!(r.class, 6);
        assert_eq!(&r.char_name[..9], b"Shadow_1\0");
        assert_eq!(r.char_name[9..], [0; 7]);
        assert_eq!(r.flags, CREATE_FLAGS_EXPANSION);
        assert_eq!(r.game_type, GAME_TYPE);
    }
}
