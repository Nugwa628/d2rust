// D2RUST's own: dev tools.
//! The dev overlay: a panel with the switch states, the replies and a
//! console line, drawn with Bevy UI nodes over the window (never into the
//! composed frame). The console ([`Console`]) is plain state;
//! the systems only feed it keys, send its lines down the channel and
//! show the replies. Nothing here decides an outcome: every command runs
//! on the server ([`super::server`]).

use std::collections::VecDeque;

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::ButtonState;
use bevy::prelude::*;
use d2_proto::dev::{self, DevCommand, DevReply, DevSwitchState};

use std::sync::Arc;

use super::info::{bug_report, DevContext, InfoOverlay};
use super::panel::{spawn_left, spawn_right, DevButton, Field, PanelAction, PanelState};
use super::server::{DevCatalog, DevChannel};
use super::slots;
use crate::bridge::BridgeResource;

/// Lines of history kept.
pub const HISTORY: usize = 50;
/// Reply lines shown.
pub const SHOWN: usize = 8;

/// A key the console takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleKey {
    Char(char),
    Backspace,
    Enter,
    Up,
    Down,
    Tab,
}

/// What a key did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleAction {
    None,
    /// A command to send.
    Send(DevCommand),
    /// One of the console's own commands (`info`, `animation`, `bugreport`,
    /// `save`, `load`): the line, run by the client.
    Local(String),
}

/// The console's state.
#[derive(Clone, Debug, Default)]
pub struct Console {
    pub input: String,
    pub history: Vec<String>,
    /// Where Up / Down stands in the history (`None`: a new line).
    pos: Option<usize>,
    /// (ok, text), newest last.
    pub lines: VecDeque<(bool, String)>,
    pub switches: DevSwitchState,
}

impl Console {
    pub fn say(&mut self, ok: bool, text: impl Into<String>) {
        self.lines.push_back((ok, text.into()));
        while self.lines.len() > SHOWN {
            self.lines.pop_front();
        }
    }

    pub fn reply(&mut self, r: DevReply) {
        self.switches = r.switches;
        self.say(r.ok, r.text);
    }

    pub fn key(&mut self, k: ConsoleKey) -> ConsoleAction {
        match k {
            ConsoleKey::Char(c) if !c.is_control() => {
                if self.input.len() < 120 {
                    self.input.push(c);
                }
            }
            ConsoleKey::Char(_) => {}
            ConsoleKey::Backspace => {
                self.input.pop();
            }
            ConsoleKey::Up => {
                if !self.history.is_empty() {
                    let i = self
                        .pos
                        .map_or(self.history.len() - 1, |i| i.saturating_sub(1));
                    self.pos = Some(i);
                    self.input = self.history[i].clone();
                }
            }
            ConsoleKey::Down => match self.pos {
                Some(i) if i + 1 < self.history.len() => {
                    self.pos = Some(i + 1);
                    self.input = self.history[i + 1].clone();
                }
                Some(_) => {
                    self.pos = None;
                    self.input.clear();
                }
                None => {}
            },
            ConsoleKey::Tab => {
                let typed = self.input.trim_start();
                if typed.contains(' ') {
                    return ConsoleAction::None;
                }
                match dev::complete(typed).as_slice() {
                    [] => {}
                    [one] => self.input = format!("{one} "),
                    many => {
                        let m = many.join(" ");
                        self.say(true, m);
                    }
                }
            }
            ConsoleKey::Enter => {
                let line = std::mem::take(&mut self.input);
                self.pos = None;
                let line = line.trim().to_string();
                if line.is_empty() {
                    return ConsoleAction::None;
                }
                if self.history.last() != Some(&line) {
                    self.history.push(line.clone());
                    if self.history.len() > HISTORY {
                        self.history.remove(0);
                    }
                }
                self.say(true, format!("> {line}"));
                match dev::parse(&line) {
                    Ok(Some(cmd)) => return ConsoleAction::Send(cmd),
                    Ok(None) if line.eq_ignore_ascii_case("clear") => self.lines.clear(),
                    Ok(None) => return ConsoleAction::Local(line),
                    Err(e) => self.say(false, e),
                }
            }
        }
        ConsoleAction::None
    }
}

/// The overlay's resource.
#[derive(Resource)]
pub struct DevOverlay {
    pub open: bool,
    pub console: Console,
    pub chan: DevChannel,
    /// The clickable panel's own state.
    pub panel: PanelState,
    /// The names the panel lists (the server resolves the commands).
    pub catalog: Arc<DevCatalog>,
    /// The console's own commands waiting for [`run_local`].
    pub local: Vec<String>,
}

impl DevOverlay {
    /// Queues commands on the channel.
    fn send(&mut self, cmds: Vec<DevCommand>) {
        if cmds.is_empty() {
            return;
        }
        let sent = match self.chan.lock() {
            Ok(mut q) => {
                q.commands.extend(cmds);
                true
            }
            Err(_) => false,
        };
        if !sent {
            self.console.say(false, "the server end is gone");
        }
    }

    /// Runs a console line made by a panel button, echoed like a typed one.
    fn run_line(&mut self, line: &str) {
        self.console.say(true, format!("> {line}"));
        match dev::parse(line) {
            Ok(Some(c)) => self.send(vec![c]),
            Ok(None) => self.local.push(line.to_string()),
            Err(e) => self.console.say(false, e),
        }
    }
}

#[derive(Component)]
struct Panel;

#[derive(Component)]
struct PanelText;

/// The rebuilt parts of the panel (the buttons): left, right.
#[derive(Component)]
struct Controls(bool);

/// Installs the overlay with the channel's client end (only `play::run` with
/// the tools on).
pub fn add_overlay(
    app: &mut impl crate::app::session::Setup,
    chan: DevChannel,
    catalog: DevCatalog,
) {
    let mut console = Console::default();
    console.say(
        true,
        format!(
            "D2RUST dev tools (channel v{}): click a button or type help; ` or Esc closes",
            dev::DEV_VERSION
        ),
    );
    app.register("dev-overlay", register_overlay);
    let w = app.world_mut();
    w.insert_resource(DevOverlay {
        open: false,
        console,
        chan,
        panel: PanelState::default(),
        catalog: Arc::new(catalog),
        local: Vec::new(),
    });
    w.init_resource::<InfoOverlay>();
    w.init_resource::<DevRelaunch>();
}

pub fn register_overlay(app: &mut App) {
    let on = resource_exists::<DevOverlay>;
    app.add_systems(
        PreUpdate,
        (
            dev_keys.after(bevy::input::InputSystems),
            dev_clicks.after(bevy::ui::UiSystems::Focus),
        )
            .run_if(on),
    )
    .add_systems(
        Update,
        (
            (spawn_panel, super::info::spawn_info).run_if(resource_added::<DevOverlay>),
            take_replies,
            run_local,
            rebuild,
            draw_panel,
            super::info::draw_info,
        )
            .chain()
            .run_if(on),
    );
}

fn spawn_panel(mut commands: Commands) {
    commands
        .spawn((
            Panel,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                padding: UiRect::all(Val::Px(4.0)),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.85)),
            GlobalZIndex(2000),
            Visibility::Hidden,
            crate::app::session::GameEntity,
        ))
        .with_children(|p| {
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                width: Val::Percent(100.0),
                ..default()
            })
            .with_children(|p| {
                p.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    width: Val::Percent(30.0),
                    ..default()
                })
                .with_children(|p| {
                    p.spawn((
                        Controls(true),
                        Node {
                            width: Val::Percent(100.0),
                            ..default()
                        },
                    ));
                    p.spawn((
                        PanelText,
                        Text::new(""),
                        TextFont {
                            font_size: bevy::text::FontSize::Px(12.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.85, 0.95, 0.85)),
                        Node {
                            margin: UiRect::all(Val::Px(4.0)),
                            ..default()
                        },
                    ));
                });
                p.spawn((
                    Controls(false),
                    Node {
                        flex_direction: FlexDirection::Row,
                        width: Val::Percent(70.0),
                        ..default()
                    },
                ));
            });
        });
}

/// The toggle key, and while open every key to the focused field and none
/// to the game.
fn dev_keys(
    mut overlay: ResMut<DevOverlay>,
    mut events: MessageReader<KeyboardInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
) {
    let was_open = overlay.open;
    let mut sends = Vec::new();
    let mut lines = Vec::new();
    for k in events.read() {
        if k.state != ButtonState::Pressed {
            continue;
        }
        if k.key_code == KeyCode::Backquote {
            overlay.open = !overlay.open;
            continue;
        }
        if !overlay.open {
            continue;
        }
        if k.key_code == KeyCode::Escape {
            overlay.open = false;
            continue;
        }
        let o = &mut *overlay;
        if o.panel.focus != Field::Console {
            match (&k.logical_key, k.key_code) {
                (_, KeyCode::Backspace) => o.panel.backspace(),
                (_, KeyCode::Enter | KeyCode::NumpadEnter) => {
                    if o.panel.focus == Field::Level {
                        lines.extend(o.panel.press(&PanelAction::SetLevel));
                    }
                    o.panel.focus = Field::Console;
                }
                (_, KeyCode::Tab) => o.panel.focus = Field::Console,
                (Key::Character(_), _) | (Key::Space, _) => {
                    for c in k.text.as_deref().unwrap_or_default().chars() {
                        o.panel.type_char(c);
                    }
                }
                _ => {}
            }
            continue;
        }
        let key = match (&k.logical_key, k.key_code) {
            (_, KeyCode::Enter | KeyCode::NumpadEnter) => ConsoleKey::Enter,
            (_, KeyCode::Backspace) => ConsoleKey::Backspace,
            (_, KeyCode::ArrowUp) => ConsoleKey::Up,
            (_, KeyCode::ArrowDown) => ConsoleKey::Down,
            (_, KeyCode::Tab) => ConsoleKey::Tab,
            (Key::Character(_), _) | (Key::Space, _) => {
                for c in k.text.as_deref().unwrap_or_default().chars() {
                    o.console.key(ConsoleKey::Char(c));
                }
                continue;
            }
            _ => continue,
        };
        match o.console.key(key) {
            ConsoleAction::Send(c) => sends.push(c),
            ConsoleAction::Local(l) => o.local.push(l),
            ConsoleAction::None => {}
        }
    }
    overlay.send(sends);
    for l in lines {
        overlay.run_line(&l);
    }
    // While open (and on the frame it opened or closed) the game sees no
    // key.
    if overlay.open || was_open {
        keys.reset_all();
    }
}

/// The panel's buttons; while the overlay is open no mouse button reaches
/// the game.
fn dev_clicks(
    mut overlay: ResMut<DevOverlay>,
    pressed: Query<(&Interaction, &DevButton), Changed<Interaction>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
) {
    if !overlay.open {
        return;
    }
    let actions: Vec<PanelAction> = pressed
        .iter()
        .filter(|(i, _)| **i == Interaction::Pressed)
        .map(|(_, b)| b.0.clone())
        .collect();
    for a in actions {
        if let Some(line) = overlay.panel.press(&a) {
            overlay.run_line(&line);
        }
    }
    // Keep held state so Bevy can generate just_released on the next
    // physical release and reset the button Interaction before another click.
    // The world input edge separately blocks input while this overlay is open.
    mouse.clear();
}

/// A `load`: the slot folder the game starts again from once this app
/// ended (`play::run`).
/// Shared with `play::run` (the app's world is gone once `App::run`
/// returned).
#[derive(Resource, Clone, Default)]
pub struct DevRelaunch(pub Arc<std::sync::Mutex<Option<std::path::PathBuf>>>);

/// The console's own commands: run on the client.
#[allow(clippy::too_many_arguments)]
fn run_local(
    mut overlay: ResMut<DevOverlay>,
    mut info: ResMut<InfoOverlay>,
    relaunch: Res<DevRelaunch>,
    ctx: Option<Res<DevContext>>,
    saver: Option<Res<crate::app::save::SaveHandle>>,
    bridge: Option<Res<BridgeResource>>,
    frames: Option<Res<crate::app::ui::AnimationFrames>>,
    view: Option<Res<crate::world_view::present::WorldViewState>>,
    mut exit: MessageWriter<AppExit>,
    mut commands: Commands,
) {
    let lines = std::mem::take(&mut overlay.local);
    for line in lines {
        let (kw, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        let rest = rest.trim();
        let r: Result<String, String> = match (kw.to_ascii_lowercase().as_str(), ctx.as_deref()) {
            ("info", _) => {
                info.on = !info.on;
                Ok(format!("info {}", if info.on { "on" } else { "off" }))
            }
            ("animation", _) => match frames.as_deref() {
                Some(handle) => {
                    crate::world_view::unit_rules::playback::dev_animation(rest, &handle.0)
                }
                None => Err("animation: no unit frames".into()),
            },
            (_, None) => Err("no game context".into()),
            ("bugreport", Some(ctx)) => match bridge.as_deref() {
                Some(b) => bug_report(&mut commands, ctx, b, view.as_deref(), rest)
                    .map(|d| format!("bug report: {}", d.display()))
                    .map_err(|e| format!("bugreport: {e}")),
                None => Err("bugreport: no game".into()),
            },
            ("save", Some(ctx)) => save_slot(
                ctx,
                saver.as_deref(),
                bridge.as_deref(),
                &overlay.console,
                rest,
            ),
            ("load", Some(ctx)) if rest.is_empty() => {
                let v = slots::list_slots(&ctx.dir);
                Ok(if v.is_empty() {
                    "no slots yet (save <name>)".into()
                } else {
                    format!("slots: {}", v.join(" "))
                })
            }
            ("load", Some(ctx)) => slots::check_name(rest)
                .and_then(|()| slots::read_slot(&slots::slot_dir(&ctx.dir, rest)))
                .and_then(|_| {
                    ctx.save_path
                        .as_ref()
                        .map(|_| ())
                        .ok_or_else(|| "load: this game has no save file".to_string())
                })
                .map(|()| {
                    if let Ok(mut r) = relaunch.0.lock() {
                        *r = Some(slots::slot_dir(&ctx.dir, rest));
                    }
                    exit.write(AppExit::Success);
                    format!("loading slot {rest}: the game restarts")
                }),
            (k, _) => Err(format!("{k}: not a console command")),
        };
        match r {
            Ok(t) => overlay.console.say(true, t),
            Err(e) => overlay.console.say(false, e),
        }
    }
}

/// `save <name>`: the server writes the character, then the
/// slot keeps a copy and the place.
fn save_slot(
    ctx: &DevContext,
    saver: Option<&crate::app::save::SaveHandle>,
    bridge: Option<&BridgeResource>,
    console: &Console,
    name: &str,
) -> Result<String, String> {
    slots::check_name(name)?;
    let saver = saver.ok_or("save: this game has no save file")?;
    let b = bridge.ok_or("save: no game")?;
    let w = b.0.world();
    let (Some(u), Some(r)) = (w.local(), w.local_room()) else {
        return Err("save: not in game".into());
    };
    saver.save().map_err(|e| format!("save: {e}"))?;
    let (x, y) = u.cell();
    let slot = slots::Slot {
        seed: ctx.seed,
        difficulty: ctx.difficulty,
        start: slots::DevStart {
            level: u32::from(r.level),
            x: i32::from(x),
            y: i32::from(y),
            switches: console.switches,
        },
    };
    let dir = slots::write_slot(&ctx.dir, name, saver.path(), &slot)?;
    Ok(format!("saved slot {name}: {}", dir.display()))
}

fn take_replies(mut overlay: ResMut<DevOverlay>) {
    let replies = match overlay.chan.lock() {
        Ok(mut q) => std::mem::take(&mut q.replies),
        Err(_) => return,
    };
    for r in replies {
        overlay.console.reply(r);
    }
}

/// Rebuilds the buttons when the panel's state or the switches changed.
fn rebuild(
    overlay: Res<DevOverlay>,
    mut shown: Local<Option<(PanelState, DevSwitchState)>>,
    controls: Query<(Entity, &Controls)>,
    mut commands: Commands,
) {
    if overlay.is_added() {
        // A new game's panel: its buttons are built again.
        *shown = None;
    }
    if !overlay.open {
        return;
    }
    let now = (overlay.panel.clone(), overlay.console.switches);
    if shown.as_ref() == Some(&now) {
        return;
    }
    for (e, c) in &controls {
        commands.entity(e).despawn_children().with_children(|p| {
            if c.0 {
                spawn_left(p, &now.0, now.1, &overlay.catalog);
            } else {
                spawn_right(p, &now.0, now.1, &overlay.catalog);
            }
        });
    }
    *shown = Some(now);
}

fn draw_panel(
    overlay: Res<DevOverlay>,
    bridge: Option<Res<BridgeResource>>,
    mut panel: Query<&mut Visibility, With<Panel>>,
    mut text: Query<&mut Text, With<PanelText>>,
    mut cursor: Query<&mut bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>,
    hw_cursor: Option<Res<crate::world_view::hw_cursor::HardwareCursorShown>>,
) {
    let want = if overlay.open {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut panel {
        if *v != want {
            *v = want;
        }
    }
    // The pointer shows while the overlay is open, and otherwise whenever
    // the game's cursor is the window's hardware cursor
    // (`world_view::hw_cursor`); else the game draws it in the frame.
    let visible = overlay.open || hw_cursor.is_some_and(|s| s.0);
    for mut c in &mut cursor {
        if c.visible != visible {
            c.visible = visible;
        }
    }
    if !overlay.open {
        return;
    }
    let c = &overlay.console;
    let mut out = String::new();
    if let Some(b) = bridge {
        let w = b.0.world();
        if let (Some(u), Some(r)) = (w.local(), w.local_room()) {
            let (x, y) = u.cell();
            out.push_str(&format!("level {}  at ({x}, {y})\n", r.level));
        }
    }
    for (ok, l) in &c.lines {
        out.push_str(if *ok { "  " } else { "! " });
        out.push_str(l);
        out.push('\n');
    }
    let cur = if overlay.panel.focus == Field::Console {
        "_"
    } else {
        ""
    };
    out.push_str(&format!("> {}{cur}", c.input));
    for mut t in &mut text {
        if t.0 != out {
            t.0 = out.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(c: &mut Console, s: &str) {
        for ch in s.chars() {
            c.key(ConsoleKey::Char(ch));
        }
    }

    #[test]
    fn repeated_button_clicks_survive_focus_release_without_reopening() {
        use bevy::input::touch::Touches;
        use bevy::ui::{ui_focus_system, UiStack};

        let chan = DevChannel::default();
        let mut app = App::new();
        app.init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<Touches>()
            .init_resource::<UiStack>()
            .insert_resource(DevOverlay {
                open: true,
                console: Console::default(),
                chan: chan.clone(),
                panel: PanelState::default(),
                catalog: Arc::new(DevCatalog::default()),
                local: Vec::new(),
            })
            .add_systems(Update, (ui_focus_system, dev_clicks).chain());
        let button = app
            .world_mut()
            .spawn((
                Node::default(),
                Interaction::None,
                DevButton(PanelAction::Run("skillpts 1".into())),
            ))
            .id();
        app.update();
        for count in 1..=3 {
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left);
            // The pointer has picked the same button. Focus only marks a
            // changed interaction when it is no longer already Pressed.
            app.world_mut()
                .entity_mut(button)
                .get_mut::<Interaction>()
                .unwrap()
                .set_if_neq(Interaction::Pressed);
            app.update();
            assert_eq!(chan.lock().unwrap().commands.len(), count);
            app.update();
            assert_eq!(
                chan.lock().unwrap().commands.len(),
                count,
                "holding does not repeat"
            );
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .release(MouseButton::Left);
            app.update();
            assert_eq!(
                *app.world().entity(button).get::<Interaction>().unwrap(),
                Interaction::None
            );
            assert!(app.world().resource::<DevOverlay>().open);
        }
    }

    #[test]
    fn enter_sends_history_walks_tab_completes() {
        let mut c = Console::default();
        typed(&mut c, "god");
        assert_eq!(
            c.key(ConsoleKey::Enter),
            ConsoleAction::Send(DevCommand::God(None))
        );
        assert!(c.input.is_empty());
        typed(&mut c, "level 7");
        assert_eq!(
            c.key(ConsoleKey::Enter),
            ConsoleAction::Send(DevCommand::Level(7))
        );
        c.key(ConsoleKey::Up);
        assert_eq!(c.input, "level 7");
        c.key(ConsoleKey::Up);
        assert_eq!(c.input, "god");
        c.key(ConsoleKey::Up);
        assert_eq!(c.input, "god");
        c.key(ConsoleKey::Down);
        assert_eq!(c.input, "level 7");
        c.key(ConsoleKey::Down);
        assert_eq!(c.input, "");
        // Tab: one match completes, several are listed.
        typed(&mut c, "fre");
        c.key(ConsoleKey::Tab);
        assert_eq!(c.input, "freemana ");
        c.input.clear();
        typed(&mut c, "he");
        c.key(ConsoleKey::Tab);
        assert_eq!(c.input, "he");
        assert_eq!(c.lines.back().unwrap().1, "help heal");
        // A bad line is answered here, nothing sent.
        c.input.clear();
        typed(&mut c, "fly");
        assert_eq!(c.key(ConsoleKey::Enter), ConsoleAction::None);
        assert!(!c.lines.back().unwrap().0);
        // `clear` empties the lines.
        typed(&mut c, "clear");
        assert_eq!(c.key(ConsoleKey::Enter), ConsoleAction::None);
        assert!(c.lines.is_empty());
        backspace_and_length(&mut c);
    }

    fn backspace_and_length(c: &mut Console) {
        typed(c, "ab");
        c.key(ConsoleKey::Backspace);
        assert_eq!(c.input, "a");
        typed(c, &"x".repeat(500));
        assert_eq!(c.input.len(), 120);
    }

    #[test]
    fn replies_keep_the_last_lines_and_the_switches() {
        let mut c = Console::default();
        for i in 0..20 {
            c.say(true, format!("{i}"));
        }
        assert_eq!(c.lines.len(), SHOWN);
        assert_eq!(c.lines.front().unwrap().1, (20 - SHOWN).to_string());
        let mut r = DevReply::ok("god on");
        r.switches.god = true;
        c.reply(r);
        assert!(c.switches.god);
        assert_eq!(c.lines.back().unwrap().1, "god on");
    }
}
