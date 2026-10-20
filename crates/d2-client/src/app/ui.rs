//! The play mode's UI: the game's panels ([`OriginalUi`]) installed in the
//! world view's [`UiRoot`], the D2RUST `dev` key bindings as the action source
//! (the game's key table is `crate::controls::keymap`), and the panel art read from
//! the user's archives ([`PanelArtLoader`]) and drawn by [`PanelArtRules`]. The
//! root forwards the panels' intents through the bridge; the UI flags' open
//! mode is the camera's. A mod game's client hooks
//! (`crate::mods::ClientMods`, a resource its binary inserts first) are
//! handed to the panels here.
//!
//! Inert without game files: the art is the user's, so `play` adds the
//! UI only with `D2_GAME_DIR` (a frame that names a file no archive holds
//! is an error, never skipped).
//!
//! Units draw through [`UnitRules`] (the play preview of decision D1:
//! `world_view::unit_rules`), their COF and component files made resident
//! by [`UnitArtLoader`] before each frame. Without the unit tables in the
//! source, no unit is drawn and a log line says why.

use crate::world_view::unit_rules::animation_baked::BakedAnimations;
use crate::world_view::unit_rules::playback::{AnimationMode, SharedPackFrames};
use std::sync::Arc;

use super::config::{AnimationFrameMode, ConfigRes};

use crate::assets::game_files::GameFiles;
use bevy::prelude::*;

use super::session::Setup;
use d2_data::bin::TableFiles;
use d2_data::tables::{decode_all, Inventory};

use crate::assets::path::FileSource;
use crate::bridge::mirror::{bridge_frame, mirror_units};
use crate::bridge::BridgeResource;
use crate::controls::Preset;
use crate::ui::layout::Screen;
use crate::ui::original::{
    FontMeasure, InvArea, OriginalUi, OriginalUiError, UiConfig, CHARACTER_FONTS,
};
use crate::ui::{NoPanelRules, NoStrings, UiRoot};
use crate::world_view::panel_art::{PanelArtLoader, PanelArtRules, SharedTextColors};
use crate::world_view::ui_bind::{TextAssetLoader, TextColors};
use crate::world_view::unit_assets::{SharedUnitArt, UnitArtLoader, UnitLooks};
use crate::world_view::unit_rules::UnitRules;
use crate::world_view::ViewError;
use crate::world_view::{UiSounds, Unspecified, WorldViewState, WorldViewUi};

use super::palette::ActPalettes;
use crate::rules::shading::ShadeTables;
use d2_formats::palette::Pl2;

/// What the play mode's UI reads from the install.
pub struct UiParts {
    /// The user's archives (panel DC6 files).
    pub source: Arc<dyn FileSource>,
    /// `inventory.bin` `inv` rectangles by record; `None`: the
    /// right panels take no pointer event.
    pub inv_areas: Option<Vec<InvArea>>,
    /// `d2exp.mpq` present.
    pub expansion_installed: bool,
    /// The character panel's font tables (`FontMeasure::load` of
    /// [`CHARACTER_FONTS`]); `None`: the panel draws no text
    /// (`ui/original.rs` fallback).
    pub fonts: Option<FontMeasure>,
    /// `difficultylevels` `ResistPenalty` by difficulty
    /// (`single_player::client_resist_penalties`); `None`: an expansion
    /// game draws no resist values.
    pub resist_penalties: Option<Vec<i32>>,
}

impl UiParts {
    /// The parts of a live install: the `inventory` table of the user's
    /// `.bin` set (a load error is an error, not a fallback).
    pub fn live(archives: Arc<GameFiles>) -> Result<Self, String> {
        let inv_areas = live_inv_areas(archives.as_ref())?;
        let expansion_installed = archives.lod();
        let fonts = FontMeasure::load(archives.as_ref(), &CHARACTER_FONTS)?;
        let resist_penalties = super::single_player::client_resist_penalties(archives.as_ref())
            .map_err(|e| e.to_string())?;
        Ok(UiParts {
            source: archives.source(),
            inv_areas: Some(inv_areas),
            expansion_installed,
            fonts: Some(fonts),
            resist_penalties: Some(resist_penalties),
        })
    }
}

/// The `inv` rectangles of the user's `inventory` table, by record, the area an
/// open right panel takes pointer events in; a load error is an error, not a
/// fallback.
pub fn live_inv_areas(archives: &GameFiles) -> Result<Vec<InvArea>, String> {
    let set = d2_data::bin::load_from(archives, "eng").map_err(|e| e.to_string())?;
    let table = set.table("inventory").ok_or("inventory not loaded")?;
    let rows: Vec<Inventory> = decode_all(table).map_err(|e| e.to_string())?;
    Ok(rows.iter().map(inv_area).collect())
}

/// An `inventory.bin` record's `inv` rectangle.
pub fn inv_area(r: &Inventory) -> InvArea {
    InvArea {
        left: r.invleft as i32,
        right: r.invright as i32,
        top: r.invtop as i32,
        bottom: r.invbottom as i32,
    }
}

/// Adds the play UI to an app set up by [`super::play::add_game`]:
/// the world view's UI (root with the wired panels, `dev` bindings, art
/// loader), [`UiSounds`] for the audio frame, and [`PanelArtRules`] over
/// [`UnitRules`] over the placeholder rules, with the unit art loader
/// ([`install_unit_rules`]). The play frame's screen ([`Screen::play`]):
/// resolution mode 2 (800 × 600) unless `play --res 640x480`.
pub fn add_original_ui(app: &mut impl Setup, parts: UiParts) -> Result<(), OriginalUiError> {
    let looks = unit_looks(parts.source.as_ref());
    add_original_ui_with(app, parts, looks)
}

/// The `levels` `LevelName` keys for the waypoint rows
/// ([`OriginalUi::set_level_names`]); nothing without the play UI.
pub fn set_level_names(app: &mut impl Setup, names: Vec<String>) {
    if let Some(mut ui) = app.world_mut().get_non_send_mut::<WorldViewUi>() {
        if let Some(o) = ui.original.as_mut() {
            o.set_level_names(names);
        }
    }
}

/// The levels' waypoint indexes for the installed waypoint menu
/// ([`OriginalUi::set_waypoint_map`]); nothing without the play UI.
pub fn set_waypoint_map(app: &mut impl Setup, map: d2_sim::world::waypoints::WaypointMap) {
    if let Some(mut ui) = app.world_mut().get_non_send_mut::<WorldViewUi>() {
        if let Some(o) = ui.original.as_mut() {
            o.set_waypoint_map(map);
        }
    }
}

/// The host's registry values the UI reads at start
/// ([`OriginalUi::set_registry`]); nothing without the play UI.
pub fn set_registry(app: &mut impl Setup, r: super::registry::UiRegistry) {
    if let Some(mut ui) = app.world_mut().get_non_send_mut::<WorldViewUi>() {
        if let Some(o) = ui.original.as_mut() {
            o.set_registry(r);
        }
    }
}

/// The store prices the server host publishes for the shop panel
/// ([`OriginalUi::set_shop_prices`]); nothing without the play UI.
pub fn set_shop_prices(app: &mut impl Setup, prices: crate::ui::original::ShopPrices) {
    if let Some(mut ui) = app.world_mut().get_non_send_mut::<WorldViewUi>() {
        if let Some(o) = ui.original.as_mut() {
            o.set_shop_prices(prices);
        }
    }
}

/// The systems of the wiring below, once per app (`app::session`).
pub fn register_text_colors(app: &mut App) {
    app.add_systems(
        PreUpdate,
        push_text_colors
            .after(bridge_frame)
            .run_if(resource_exists::<BridgeResource>)
            .run_if(resource_exists::<WorldViewState>)
            .run_if(resource_exists::<ActPalettes>)
            .run_if(resource_exists::<TextColorMaps>),
    );
}

/// [`add_original_ui`] with the unit tables `looks` (instead of the ones
/// read from `parts.source`).
pub fn add_original_ui_with(
    app: &mut impl Setup,
    parts: UiParts,
    looks: UnitLooks,
) -> Result<(), OriginalUiError> {
    let config = UiConfig {
        screen: Screen::play(),
        expansion_installed: parts.expansion_installed,
    };
    let mut original = OriginalUi::new(config, parts.inv_areas)?;
    // A mod game's client hooks, when its binary inserted them.
    if let Some(mods) = app.world().get_resource::<crate::mods::ClientMods>() {
        original.set_mods(mods.clone());
    }
    // The cursor step draws on the client seed the weather and sound share.
    let link = app
        .world_mut()
        .get_resource_or_insert_with(crate::audio::driver::SoundLink::default)
        .clone();
    original.set_client_seed(link.client_seed());
    if let Some(fonts) = parts.fonts {
        original.set_fonts(fonts);
    }
    if let Some(penalties) = parts.resist_penalties {
        original.set_resist_penalties(penalties);
    }
    // `UiStates` is the authority and the root mirrors it: the
    // root's own rules are never asked.
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    original.install(&mut root)?;
    super::items::prepare_ui(app, &mut original);
    let files = original.files();
    let mut ui = WorldViewUi::new(root, Box::new(NoStrings));
    ui.original = Some(original);
    ui.bindings = crate::ui::front_end::screens::controls::saved_bindings()
        .or_else(|| Preset::Original.bindings());
    ui.art = Some(PanelArtLoader::new(parts.source.clone(), files.clone()));
    ui.text = Some(TextAssetLoader {
        source: parts.source.clone(),
    });
    app.world_mut().insert_non_send(ui);
    app.world_mut().init_resource::<UiSounds>();
    let units = install_unit_rules_with(app, parts.source, looks);
    let text = SharedTextColors::default();
    app.register("text-colors", register_text_colors);
    app.world_mut().insert_resource(TextColorMaps {
        shared: text.clone(),
        by_act: [None; 5],
        shade_by_act: [None; 5],
        shown: None,
    });
    if let Some(mut state) = app.world_mut().get_resource_mut::<WorldViewState>() {
        state.rules = Box::new(PanelArtRules {
            rules: units,
            files,
            text: Some(text),
        });
    }
    Ok(())
}

/// The PL2 text-colour maps of each act, pushed into the world view's map
/// table once per act, and the shared set the rules
/// read.
#[derive(Resource)]
pub struct TextColorMaps {
    pub shared: SharedTextColors,
    by_act: [Option<TextColors>; 5],
    /// The act's blend tables, pushed once per
    /// act: the front-end UI path has no tile feed to push them, and a UI
    /// cel of draw mode 2 reads them (`ui_cel_ops`).
    shade_by_act: [Option<ShadeTables>; 5],
    shown: Option<u8>,
}

/// The text colours of the model's palette act (act 0 until 0x03 sets
/// one, as [`ActPalettes::wanted`]); a palette without the maps is an
/// error.
pub fn push_text_colors(
    bridge: Res<BridgeResource>,
    palettes: Res<ActPalettes>,
    mut maps: ResMut<TextColorMaps>,
    mut state: ResMut<WorldViewState>,
) -> Result {
    let act = ActPalettes::wanted(bridge.0.world().palette_act);
    if maps.shown == Some(act) {
        return Ok(());
    }
    let i = if act < 5 { usize::from(act) } else { 0 };
    let colors = match maps.by_act[i] {
        Some(c) => c,
        None => {
            let c = TextColors::push(&mut state.assets.maps, palettes.of(act))?;
            maps.by_act[i] = Some(c);
            c
        }
    };
    let shades = match maps.shade_by_act[i] {
        Some(t) => t,
        None => {
            let pl2 = Pl2::parse(palettes.of(act)).map_err(|e| ViewError::Unresolved {
                what: "UI blend tables",
                spec: "render/blend-modes.md",
                message: format!("act {act} pal.pl2: {e}"),
            })?;
            let t = ShadeTables::push(&mut state.assets.maps, &pl2);
            maps.shade_by_act[i] = Some(t);
            t
        }
    };
    state.assets.shades = Some(shades);
    state.assets.text_colors = Some(colors);
    *maps.shared.write().unwrap_or_else(|e| e.into_inner()) = Some(colors);
    maps.shown = Some(act);
    Ok(())
}

/// The unit loader as a resource: [`load_unit_art`] runs it each frame.
#[derive(Resource)]
pub struct UnitArt(pub UnitArtLoader);

/// The unit rules of the play preview over [`Unspecified`], and the
/// loader system that fills their art from `source` (after the bridge
/// mirrored the frame's units, before the world view draws). The unit
/// tables come from `source`; when they do not load, the rules draw no
/// unit (D1: logged, not an error).
pub fn install_unit_rules(
    app: &mut impl Setup,
    source: Arc<dyn FileSource>,
) -> UnitRules<Unspecified, SharedPackFrames> {
    let looks = unit_looks(source.as_ref());
    install_unit_rules_with(app, source, looks)
}

/// The unit tables of `source`; when they do not load, none (logged).
fn unit_looks(source: &dyn FileSource) -> UnitLooks {
    match UnitLooks::live(source) {
        Ok(l) => l,
        Err(e) => {
            warn!("unit art: unit tables not loaded, no unit is drawn: {e}");
            UnitLooks::default()
        }
    }
}

/// The systems of the wiring below, once per app (`app::session`).
pub fn register_unit_art(app: &mut App) {
    app.add_systems(
        PreUpdate,
        load_unit_art
            .after(mirror_units)
            .run_if(resource_exists::<BridgeResource>)
            .run_if(resource_exists::<WorldViewState>)
            .run_if(resource_exists::<UnitArt>),
    );
}

/// Entry points that play baked animation frames insert this marker before
/// installing the play UI; the frames are read from the bake folder
/// (`animation_baked::dir`). Without it no baked frames load. Nothing is
/// generated while the game runs.
#[derive(Resource)]
pub struct LoadBakedAnimations;

/// The play mode's generated-frame selection, for the Video setting and the
/// dev console. With [`LoadBakedAnimations`] the [`BakedAnimations`] loader
/// fills it from the bake folder; mod games may also install their own packs
/// through the same handle.
#[derive(Resource, Clone)]
pub struct AnimationFrames(pub SharedPackFrames);

impl AnimationFrames {
    pub fn apply(&self, choice: AnimationFrameMode) {
        self.0.set_mode(match choice {
            AnimationFrameMode::Original => AnimationMode::Original,
            AnimationFrameMode::Smooth => AnimationMode::Smooth,
        });
    }
}

/// [`install_unit_rules`] with the unit tables `looks`.
pub fn install_unit_rules_with(
    app: &mut impl Setup,
    source: Arc<dyn FileSource>,
    looks: UnitLooks,
) -> UnitRules<Unspecified, SharedPackFrames> {
    let looks = Arc::new(looks);
    let art = SharedUnitArt::default();
    let frames = SharedPackFrames::default();
    // The saved Video choice, before the first loader pass can load packs.
    let choice = app
        .world()
        .get_resource::<ConfigRes>()
        .map_or(AnimationFrameMode::Smooth, |c| c.settings.animation_frames);
    let handle = AnimationFrames(frames.clone());
    handle.apply(choice);
    app.world_mut().insert_resource(handle);
    if app.world_mut().contains_resource::<LoadBakedAnimations>() {
        let dir = crate::world_view::unit_rules::animation_baked::dir();
        app.world_mut().insert_resource(BakedAnimations::new(
            source.clone(),
            looks.clone(),
            frames.clone(),
            &dir,
        ));
    }
    app.register("unit-art", register_unit_art);
    app.world_mut().insert_resource(UnitArt(UnitArtLoader {
        source,
        looks: looks.clone(),
        art: art.clone(),
    }));
    UnitRules {
        frame_source: frames,
        rules: Unspecified,
        looks,
        art,
    }
}

/// Makes the unit files of the model's units resident (D1: a failed file
/// is logged once and skipped).
pub fn load_unit_art(
    mut animation: Option<ResMut<BakedAnimations>>,
    loader: Res<UnitArt>,
    bridge: Res<BridgeResource>,
    mut state: ResMut<WorldViewState>,
) {
    for line in loader.0.ensure(bridge.0.world(), &mut state.assets) {
        warn!("{line}");
    }
    if let Some(animation) = animation.as_mut() {
        animation.update(bridge.0.world(), &loader.0.art, &mut state.assets);
    }
}

#[cfg(test)]
mod animation_tests {
    use super::*;
    use crate::assets::path::MemorySource;

    #[test]
    fn only_the_marked_entry_point_installs_the_baked_frame_loader() {
        let mut app = App::new();
        let source = Arc::new(MemorySource::default());
        install_unit_rules_with(&mut app, source.clone(), UnitLooks::default());
        assert!(!app.world().contains_resource::<BakedAnimations>());
        app.insert_resource(LoadBakedAnimations);
        install_unit_rules_with(&mut app, source, UnitLooks::default());
        assert!(app.world().contains_resource::<BakedAnimations>());
    }
}
