//! One app for the whole session (front end -> game -> front end ->...), one
//! window and one event loop. The game's systems are registered once, when the
//! app is built ([`register`] keys); a game is the resources and entities its
//! start installs ([`Setup`] on the `World`) and its end removes
//! ([`end_game`]).
//!
//! Helpers that wire the game take `&mut impl Setup`: on an [`App`] they
//! register their systems (once per app) and install their resources; on
//! a [`World`] (a game started inside the running app) they only install,
//! the systems being already there.

use std::collections::BTreeSet;

use bevy::ecs::component::ComponentId;
use bevy::prelude::*;

/// Where the game's wiring goes: an [`App`] being built (registers and
/// installs) or the running app's [`World`] (installs only).
pub trait Setup {
    fn world(&self) -> &World;
    fn world_mut(&mut self) -> &mut World;
    /// Runs `f` on the app the first time `key` is registered there; a
    /// no-op on a [`World`] (the app registered it when it was built).
    fn register(&mut self, key: &'static str, f: fn(&mut App));
}

impl Setup for App {
    fn world(&self) -> &World {
        App::world(self)
    }
    fn world_mut(&mut self) -> &mut World {
        App::world_mut(self)
    }
    fn register(&mut self, key: &'static str, f: fn(&mut App)) {
        let first = App::world_mut(self)
            .get_resource_or_insert_with(Registered::default)
            .0
            .insert(key);
        if first {
            f(self);
        }
    }
}

impl Setup for World {
    fn world(&self) -> &World {
        self
    }
    fn world_mut(&mut self) -> &mut World {
        self
    }
    fn register(&mut self, key: &'static str, _f: fn(&mut App)) {
        debug_assert!(
            self.get_resource::<Registered>()
                .is_some_and(|r| r.0.contains(key)),
            "{key}: not registered when the app was built"
        );
    }
}

/// The wiring keys an app has registered ([`Setup::register`]).
#[derive(Resource, Default)]
pub struct Registered(pub BTreeSet<&'static str>);

/// Marks an entity that belongs to one game: [`end_game`] despawns it.
#[derive(Component, Clone, Copy, Default)]
pub struct GameEntity;

/// The resources present before a game started ([`begin_game`]):
/// [`end_game`] removes every other one. The app was built and has run
/// before a game starts, so whatever appears after is the game's.
#[derive(Resource)]
pub struct GameScope {
    before: BTreeSet<ComponentId>,
}

/// Bumped by every [`end_game`]: render-world state keyed to a game
/// (the compositor's atlas and job sequence, the present counter) resets
/// when it changes.
#[derive(Resource, Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct GameGeneration(pub u64);

/// The resources (and `!Send` data) in `world`.
fn resources(world: &World) -> BTreeSet<ComponentId> {
    world
        .iter_resources()
        .map(|(info, _)| info.id())
        .chain(
            world
                .storages()
                .non_sends
                .iter()
                .filter(|(_, d)| d.is_present())
                .map(|(id, _)| id),
        )
        .collect()
}

/// Notes the resources present before a game's install: the game is what
/// comes after.
pub fn begin_game(world: &mut World) {
    let before = resources(world);
    world.insert_resource(GameScope { before });
}

/// The resources that outlive a game although a game may add them: the
/// session's own.
fn kept(world: &World, id: ComponentId) -> bool {
    use std::any::TypeId;
    let Some(t) = world.components().get_info(id).and_then(|i| i.type_id()) else {
        return false;
    };
    [
        TypeId::of::<Registered>(),
        TypeId::of::<GameScope>(),
        TypeId::of::<GameGeneration>(),
        TypeId::of::<OneApp>(),
    ]
    .contains(&t)
}

/// Ends a game: despawns its entities ([`GameEntity`], the unit mirror's
/// views), removes every resource added since [`begin_game`] and
/// resets the per-game state of the ones registered with the app. The
/// leave through the server ([`super::play::after_run`]) runs before.
pub fn end_game(world: &mut World) {
    let mut q = world.query_filtered::<Entity, With<GameEntity>>();
    let mut gone: Vec<Entity> = q.iter(world).collect();
    if let Some(mut index) = world.get_resource_mut::<crate::bridge::mirror::MirrorIndex>() {
        gone.extend(index.0.values().copied());
        index.0.clear();
    }
    for e in gone {
        if let Ok(e) = world.get_entity_mut(e) {
            e.despawn();
        }
    }
    let before = world
        .remove_resource::<GameScope>()
        .map(|s| s.before)
        .unwrap_or_default();
    for id in resources(world) {
        if before.contains(&id) || kept(world, id) {
            continue;
        }
        if world
            .storages()
            .non_sends
            .get(id)
            .is_some_and(|d| d.is_present())
        {
            world.remove_non_send_by_id(id);
        } else {
            world.remove_resource_by_id(id);
        }
    }
    // Registered with the app, filled by the game.
    reset::<crate::bridge::mirror::FrameOutputs>(world);
    reset::<crate::audio::driver::SoundLink>(world);
    reset::<crate::world_view::UiSounds>(world);
    reset::<crate::bridge::mirror::InputOnlyPass>(world);
    world.get_resource_or_insert_with(GameGeneration::default).0 += 1;
}

fn reset<R: Resource + Default>(world: &mut World) {
    if world.contains_resource::<R>() {
        world.insert_resource(R::default());
    }
}

/// What the one window shows: the front end (main menu, character select,...)
/// or a game.
#[derive(States, Default, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Screen {
    #[default]
    Front,
    Game,
}

/// The app runs the whole session ([`Screen`]): a game's way out goes back
/// to the front end instead of ending the app.
#[derive(Resource, Clone, Copy, Default)]
pub struct OneApp;

/// A game asked to go back to the front end (the hardcore death's Esc).
#[derive(Resource, Default)]
pub struct ToMenu;

/// Builds and installs a game on the running app's `World` (the session's
/// game start, `main.rs`); `Err`: the game did not start.
pub struct GameStarter(pub Box<WorldStep<()>>);

/// Ends a game on the running app's `World` once its leave ran (the save
/// and the dev tools' `load`, `main.rs`). `Ok(true)`: start the next game
/// at once (a dev `load`), else back to the front end.
pub struct GameEnder(pub Box<WorldStep<bool>>);

/// A session step on the running app's `World`.
pub type WorldStep<T> = dyn FnMut(&mut World) -> anyhow::Result<T>;

/// The session's systems: the screen state, the camera the front end and
/// the game's loading cover draw through, the game's start and end.
pub fn add_session(app: &mut App) {
    app.insert_resource(OneApp)
        .init_state::<Screen>()
        .add_systems(Startup, spawn_camera)
        .add_systems(OnEnter(Screen::Front), hide_pointer)
        .add_systems(OnEnter(Screen::Game), enter_game)
        .add_systems(
            Last,
            leave_to_menu
                .run_if(in_state(Screen::Game))
                .before(super::play::leave_on_exit),
        );
}

/// The camera that outlives every screen (the game adds its own present
/// camera on top while it runs). MSAA is off on every camera: cameras that
/// draw into the same window must share one sample count, and the game's
/// present camera has none (a mix is a GPU validation error that ends the
/// app on strict drivers).
fn spawn_camera(mut commands: Commands) {
    commands.spawn((Camera2d, bevy::render::view::Msaa::Off));
}

/// The window's cursor options with the system pointer hidden. D2RUST
/// shows only its own cursor, drawn into the picture on the menus and in
/// the game. The window starts with these options, and the session keeps
/// the pointer hidden on every screen.
pub fn hidden_cursor() -> bevy::window::CursorOptions {
    bevy::window::CursorOptions {
        visible: false,
        ..default()
    }
}

/// Shows or hides the system pointer over the primary window.
fn set_cursor(world: &mut World, visible: bool) {
    let mut q = world
        .query_filtered::<&mut bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>();
    for mut c in q.iter_mut(world) {
        if c.visible != visible {
            c.visible = visible;
        }
    }
}

/// `OnEnter(Screen::Front)`: the front end draws its own cursor, so the
/// system pointer is hidden on every visit (the first start included).
fn hide_pointer(world: &mut World) {
    set_cursor(world, false);
}

/// `OnEnter(Screen::Game)`: the game starts ([`GameStarter`]); a game
/// that fails to start leaves nothing and goes back to the front end.
pub fn enter_game(world: &mut World) {
    begin_game(world);
    let Some(mut starter) = world.remove_non_send::<GameStarter>() else {
        return;
    };
    let started = (starter.0)(world);
    world.insert_non_send(starter);
    match started {
        Ok(()) => {
            // The game draws its own cursor. The window keeps the size the
            // player chose (`config::WindowChoice`): the frame is presented
            // in it.
            set_cursor(world, false);
        }
        Err(e) => {
            error!("the game did not start: {e:#}");
            end_game(world);
            world.resource_mut::<NextState<Screen>>().set(Screen::Front);
        }
    }
}

/// The game's Save and Exit is through (the server's leave took the
/// client out of the game), or [`ToMenu`] was asked: the leave runs
/// ([`super::play::after_run`]), the game ends ([`GameEnder`],
/// [`end_game`]) and the front end opens at the main menu.
pub fn leave_to_menu(world: &mut World) {
    let asked = world.remove_resource::<ToMenu>().is_some();
    let through = world
        .get_resource::<crate::bridge::BridgeResource>()
        .is_some_and(|b| b.0.world().exit_requested && !b.0.world().in_game);
    if !asked && !through {
        return;
    }
    let left = super::play::after_run(world).map_err(|e| e.to_string());
    if let Some(r) = world.get_resource::<super::play::ExitReport>() {
        if let Ok(mut slot) = r.0.lock() {
            *slot = Some(super::play::ExitOutcome {
                to_menu: true,
                left,
            });
        }
    }
    let again = match world.remove_non_send::<GameEnder>() {
        Some(mut ender) => {
            let r = (ender.0)(world);
            world.insert_non_send(ender);
            r.unwrap_or_else(|e| {
                error!("ending the game: {e:#}");
                false
            })
        }
        None => false,
    };
    end_game(world);
    crate::world_view::hw_cursor::reset(world);
    // The front end draws its own cursor: the pointer stays hidden.
    set_cursor(world, false);
    if again {
        enter_game(world);
    } else {
        world.resource_mut::<NextState<Screen>>().set(Screen::Front);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Count(u32);

    #[derive(Resource)]
    struct Before;

    #[derive(Resource)]
    struct PerGame(#[allow(dead_code)] u32);

    struct PerGameUi;

    fn count(app: &mut App) {
        app.world_mut().resource_mut::<Count>().0 += 1;
    }

    fn pointer_visible(app: &mut App) -> bool {
        let mut q = app
            .world_mut()
            .query_filtered::<&bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>();
        q.single(app.world()).expect("one window").visible
    }

    #[test]
    fn the_front_end_hides_the_system_pointer() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin));
        // A window that shows the pointer (the platform default).
        app.world_mut().spawn((
            bevy::window::PrimaryWindow,
            bevy::window::CursorOptions {
                visible: true,
                ..default()
            },
        ));
        add_session(&mut app);
        app.update();
        assert!(!pointer_visible(&mut app), "first start on the menu");
        // A game showed its hardware cursor, then went back to the menu.
        app.world_mut()
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Game);
        app.update();
        set_cursor(app.world_mut(), true);
        app.world_mut().insert_resource(ToMenu);
        app.update();
        app.update();
        assert_eq!(
            *app.world().resource::<State<Screen>>().get(),
            Screen::Front
        );
        assert!(!pointer_visible(&mut app), "back on the menu");
        assert!(!hidden_cursor().visible);
    }

    #[test]
    fn a_key_registers_once_per_app() {
        let mut app = App::new();
        app.init_resource::<Count>();
        app.register("k", count);
        app.register("k", count);
        app.register("other", count);
        assert_eq!(app.world().resource::<Count>().0, 2);
        // A running app's world only installs.
        app.world_mut().register("k", count);
        assert_eq!(app.world().resource::<Count>().0, 2);
    }

    #[test]
    fn a_games_end_removes_what_it_added_and_nothing_else() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(Before);
        let window_like = app.world_mut().spawn_empty().id();
        for round in 0..3u64 {
            let world = app.world_mut();
            begin_game(world);
            world.insert_resource(PerGame(1));
            world.insert_non_send(PerGameUi);
            let e = world.spawn(GameEntity).id();
            app.update();
            let world = app.world_mut();
            end_game(world);
            assert!(world.get_resource::<PerGame>().is_none());
            assert!(world.get_non_send::<PerGameUi>().is_none());
            assert!(world.get_entity(e).is_err());
            assert!(world.get_resource::<Before>().is_some());
            assert!(world.get_entity(window_like).is_ok());
            assert!(
                world.get_resource::<Time>().is_some(),
                "engine resources stay"
            );
            assert_eq!(world.resource::<GameGeneration>().0, round + 1);
        }
    }
}
