//! The play app's automap: the session over the live tables, handed to
//! the world view with its draw sink (`world_view::automap_view`). The
//! session itself (act load, reveal, toggle, teardown) is
//! `ui::automap::session`; this only builds it.

use std::sync::Arc;

use bevy::prelude::*;
use d2_data::tables::{Leveldefs, Levels, Monstats, Monstats2, Objects};
use d2_server::world_data::game::GameTables;

use crate::assets::path::FileSource;
use crate::ui::automap::options::MemoryStore;
pub use crate::ui::automap::session::SaveFiles;
use crate::ui::automap::session::{AutomapSession, AutomapSource};
use crate::ui::automap::{Automap, AutomapLevels, CelPicker, UnitCels, SEED};
use crate::world_view::automap_view::{cel_limits, AutomapView};
use crate::world_view::WorldViewState;

/// `Leveldefs` `Layer` / `LevelType` and `Levels` `Act` per level id.
struct LevelFacts {
    layer: Vec<u32>,
    level_type: Vec<u32>,
    act: Vec<u8>,
}

impl AutomapLevels for LevelFacts {
    fn layer(&self, level: u32) -> Option<u32> {
        self.layer.get(level as usize).copied()
    }
    fn level_type(&self, level: u32) -> Option<u32> {
        self.level_type.get(level as usize).copied()
    }
    fn act(&self, level: u32) -> Option<u8> {
        self.act.get(level as usize).copied()
    }
}

/// `monstats2` `automapCel` through `monstats` `MonStatsEx`, and the
/// `objects` `AutoMap` column.
struct LiveCels {
    monster: Vec<Option<u32>>,
    object: Vec<u32>,
}

impl UnitCels for LiveCels {
    fn monster_cel(&self, class: u32) -> Option<u32> {
        self.monster.get(class as usize).copied().flatten()
    }
    fn object_cel(&self, class: u32) -> Option<u32> {
        self.object.get(class as usize).copied()
    }
}

/// The session's tables from the loaded game tables.
pub fn live_source(t: &GameTables) -> Result<AutomapSource, String> {
    let e = |e: d2_server::world_data::WorldDataError| e.to_string();
    let defs = t.rows::<Leveldefs>().map_err(e)?;
    let levels = t.rows::<Levels>().map_err(e)?;
    let stats = t.rows::<Monstats>().map_err(e)?;
    let stats2 = t.rows::<Monstats2>().map_err(e)?;
    let objects = t.rows::<Objects>().map_err(e)?;
    Ok(AutomapSource {
        picker: CelPicker::new(&t.fixed.automap),
        levels: Box::new(LevelFacts {
            layer: defs.iter().map(|d| d.layer).collect(),
            level_type: defs.iter().map(|d| d.leveltype).collect(),
            act: levels.iter().map(|l| l.act).collect(),
        }),
        cels: Box::new(LiveCels {
            monster: stats
                .iter()
                .map(|m| stats2.get(usize::from(m.monstatsex)).map(|s| s.automapcel))
                .collect(),
            object: objects.iter().map(|o| o.automap).collect(),
        }),
    })
}

/// Gives the world view an automap over `source` (files in `files`, or
/// memory only) and its cel files from `cels`. Tab then toggles it.
pub fn add_automap(
    app: &mut impl crate::app::session::Setup,
    source: AutomapSource,
    files: Option<SaveFiles>,
    cels: Arc<dyn FileSource>,
    expansion: bool,
) {
    // The options registry seam is in memory in the preview.
    let mut map = Automap::new(&MemoryStore::default(), SEED);
    // The cel files load at UI init (full size), and
    // with them the cel counts the file load checks.
    map.limits = cel_limits(cels.as_ref(), map.view.mini, expansion);
    let mut state = app.world_mut().resource_mut::<WorldViewState>();
    state.automap = Some(AutomapSession::new(map, source, files));
    state.automap_view = Some(AutomapView::new(cels, expansion));
}
