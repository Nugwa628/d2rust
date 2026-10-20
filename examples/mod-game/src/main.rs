//! The example mod game's binary: opens the player's files with its own
//! layers, builds a game under its own rules with its own save folder,
//! joins a new character and runs a few headless frames
//! (`docs/mods/mod-game.md`). A real mod game starts its window the same
//! way D2RUST's exe does, with its own builder.

use mod_game_example::{builder, rules_name, run_frames, start_and_join, Options, StepClock};

fn main() -> anyhow::Result<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let data = d2_client::app::single_player::GameData::open(&options.source)?;
    let clock = StepClock::default();
    let (mut link, _) = start_and_join(builder(data, options.saves.clone()), &clock)?;
    let ticks = run_frames(&mut link, &clock, 25)?;
    println!(
        "{}: {ticks} ticks, saves in {}",
        rules_name(&mut link)?.unwrap_or_else(|| "D2RUST".into()),
        options.saves.display()
    );
    Ok(())
}
