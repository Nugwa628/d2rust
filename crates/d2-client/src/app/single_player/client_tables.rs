//! Presentation adapters for the server-owned table loader.
use super::*;
use crate::bridge::drlg::DrlgSource;
use crate::bridge::world::UnitRows;
pub use d2_server::game::client_tables::*;
use d2_sim::drlg::maze::Maze;
use d2_sim::items::inventory::tables::InvTables;
use d2_sim::wiring::worldgen::levels::WorldTypes;
/// What the client DRLG copy is built from: the same table view, tile headers
/// and level-type data the game's DRLG reads, with fresh level-type state for
/// every client act (the client never reads the server's DRLG).
pub fn client_drlg_source(data: &GameData) -> DrlgSource {
    let GameData::Live(d) = data;
    let live = d.clone();
    let drlg = Arc::new(d.levels.drlg.clone());
    let types_data = drlg.clone();
    DrlgSource {
        data: drlg,
        tiles: Arc::new(d.files.dt1.clone()),
        types: Arc::new(move || {
            Box::new(WorldTypes::new(
                types_data.clone(),
                Maze::new(live.levels.maze.clone()),
                live.levels.preset.clone(),
                live.levels.outdoor.clone(),
                Box::new(LiveDs1(live.clone())),
                Box::new(LiveSubs(live.clone())),
            ))
        }),
    }
}

/// Adapt the loaded class speeds to the local prediction model.
pub fn walk_speeds(
    data: &GameData,
    character: &Character,
) -> Result<Option<crate::bridge::predict::Speeds>, BuildError> {
    Ok(
        d2_server::game::client_tables::walk_velocities(data, character)?
            .map(|(walk, run)| crate::bridge::predict::Speeds { walk, run }),
    )
}
/// The client player update's animation lookup
/// ([`crate::app::anim_names::ClientPlayerAnims`]): the user's `AnimData.d2`,
/// the art's token tables and the items tables.
pub fn client_player_anims(
    data: &GameData,
) -> Result<crate::app::anim_names::ClientPlayerAnims, BuildError> {
    let GameData::Live(d) = data;
    let looks = crate::world_view::unit_assets::UnitLooks::live(d.archives.as_ref())
        .map_err(BuildError::Tables)?;
    let inv = InvTables::from_fixed(&d.tables.fixed)
        .map_err(|e| BuildError::Tables(format!("inventory tables: {e}")))?;
    Ok(crate::app::anim_names::ClientPlayerAnims::new(
        Arc::new(looks),
        Arc::new(d.tables.anim.clone()),
        &inv,
    ))
}

/// Populate client animation rows using the established presentation table.
pub fn client_monster_anims(
    archives: &crate::assets::game_files::GameFiles,
    rows: &mut UnitRows,
) -> Result<(), BuildError> {
    d2_server::game::client_tables::client_monster_anims_with(archives, rows, || {
        crate::world_view::unit_assets::UnitLooks::live(archives).map_err(BuildError::Tables)
    })
}
