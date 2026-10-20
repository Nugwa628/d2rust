//! The front-end host's screen hookups on synthetic data, no game files:
//! credits rows, create (hover, hero frames, stub `.d2s`), controls, and
//! the char-select list from a save folder.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use d2_client::app::front_host::{
    add_front_end, register_credits, write_stub, DirSaves, FrontArt, FrontHost,
};
use d2_client::app::front_start::{registry, StartHandles};
use d2_client::assets::path::MemorySource;
use d2_client::ui::front_end::screens::create::{Class, NewCharacter, EXPANSION, HERO_H, HERO_W};
use d2_client::ui::front_end::*;
use d2_client::ui::geom::Point;
use d2_formats::d2s::{self, ReadOptions, SaveTables, StatSave};

fn app(host: FrontHost) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::asset::AssetPlugin::default(),
        bevy::input::InputPlugin,
    ))
    .init_asset::<Image>()
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
        40,
    )));
    add_front_end(&mut app, host);
    app
}

/// The host as `main` builds it: the start-flow registry, the credits text
/// from `art`, the stub writer.
fn host_for(dir: &std::path::Path, saves: bool, art: Option<FrontArt>) -> FrontHost {
    let handles = StartHandles::default();
    let mut reg = registry(dir, &handles);
    if let Some(a) = &art {
        register_credits(&mut reg, a);
    }
    let saves: Box<dyn SaveFolder> = if saves {
        Box::new(DirSaves(dir.to_path_buf()))
    } else {
        Box::new(false)
    };
    let front = FrontEnd::new(true, saves, reg);
    FrontHost::with_front(front, art, false).with_stub_writer(handles.created, dir.to_path_buf())
}

fn front(a: &mut App) -> Mut<'_, FrontHost> {
    a.world_mut().non_send_mut::<FrontHost>()
}

fn ticks(a: &mut App, n: u32) {
    for _ in 0..n {
        a.update();
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("d2rust-fhs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn click(a: &mut App, p: Point) {
    front(a).front.input(FrontInput::Down(p));
    front(a).front.input(FrontInput::Up(p));
    ticks(a, 2);
}

struct NoTables;
impl SaveTables for NoTables {
    fn stat_save(&self, _: u16) -> Option<StatSave> {
        None
    }
    fn item_entry_len(&self, _: &[u8]) -> Result<usize, String> {
        Err("none".into())
    }
}

#[test]
fn credits_rows_are_drawn_after_ticks() {
    let mut mem = MemorySource::default();
    mem.insert(
        r"data\local\ui\eng\ExpansionCredits.txt",
        b"*Heads\r\nAl\r\nBo\r\n".to_vec(),
    );
    let art = FrontArt::new(Arc::new(mem));
    let host = host_for(&temp_dir("credits"), false, Some(art));
    let mut a = app(host);
    front(&mut a).front.trigger(Trigger::Credits);
    assert_eq!(front(&mut a).front.current(), CREDITS);
    let mut seen = false;
    for _ in 0..1500 {
        a.update();
        let h = a.world().non_send::<FrontHost>();
        seen |= h
            .drawn
            .iter()
            .any(|d| matches!(d, DrawItem::Text { text, .. } if text == "Heads"));
        if seen {
            break;
        }
    }
    assert!(seen, "credits heading never drawn");
}

#[test]
fn create_hover_frames_and_stub_save() {
    let dir = temp_dir("create");
    let host = host_for(&dir, false, None);
    let mut a = app(host);
    front(&mut a).front.trigger(Trigger::SinglePlayer);
    assert_eq!(front(&mut a).front.current(), CHAR_CREATE);
    let (class, x, y) = EXPANSION[0];

    // Hover: the hero's hover animation file and the class texts appear.
    let over = Point::new(x + i32::from(HERO_W) / 2, y - i32::from(HERO_H) / 2);
    front(&mut a).front.input(FrontInput::Move(over));
    ticks(&mut a, 3);
    let drawn = a.world().non_send::<FrontHost>().drawn.clone();
    let hover_file = d2_client::ui::front_end::screens::create::anim_file(class, 1);
    assert!(drawn
        .iter()
        .any(|d| matches!(d, DrawItem::Art { file, .. } if *file == hover_file)));
    assert!(drawn
        .iter()
        .any(|d| matches!(d, DrawItem::Text { string_id, .. } if *string_id == class.strings().0)));

    // Select, type a name, OK.
    click(&mut a, over);
    for c in "Zed".encode_utf16() {
        front(&mut a).front.input(FrontInput::Char(c));
    }
    ticks(&mut a, 2);
    click(&mut a, Point::new(640, 560));
    let h = a.world().non_send::<FrontHost>();
    assert!(
        matches!(h.outcome, Some(Outcome::GameLoad(g)) if g.new_character),
        "{:?}",
        h.outcome
    );
    let path = h.created.clone().expect("stub written").unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len(), 335);
    let s = d2s::read(
        &bytes,
        &ReadOptions {
            expansion: true,
            game: None,
        },
        &NoTables,
    )
    .unwrap();
    assert_eq!(s.header.name_bytes(), b"Zed");
    assert_eq!(s.header.class, class.id());
    assert!(s.body.is_none());
    // The duplicate-name check now sees the file.
    assert!(d2_client::ui::front_end::screens::create::name_taken_in(
        &dir, "zed"
    ),);
}

#[test]
fn char_select_lists_a_saved_character() {
    let dir = temp_dir("select");
    let c = NewCharacter {
        name: "Bolt".into(),
        class: Class::Sorceress,
        hardcore: false,
        expansion: true,
    };
    write_stub(&dir, &c, 1).unwrap();
    let host = host_for(&dir, true, None);
    let mut a = app(host);
    front(&mut a).front.trigger(Trigger::SinglePlayer);
    assert_eq!(front(&mut a).front.current(), CHAR_SELECT);
    let f = &front(&mut a).front;
    assert!(
        f.controls()
            .iter()
            .any(|c| c.text.as_deref().is_some_and(|t| t.contains("Bolt"))),
        "no control names the saved character"
    );
}

#[test]
fn controls_screen_is_drawn_and_takes_wheel() {
    let host = host_for(&temp_dir("controls"), false, None);
    let mut a = app(host);
    front(&mut a).front.goto(CONTROLS);
    ticks(&mut a, 2);
    let drawn = a.world().non_send::<FrontHost>().drawn.clone();
    assert!(drawn.iter().any(|d| matches!(d, DrawItem::Rect { .. })));
    assert!(drawn.iter().any(|d| matches!(d, DrawItem::Text { .. })));
    let first = drawn.clone();
    front(&mut a).front.input(FrontInput::Wheel(-120));
    ticks(&mut a, 1);
    let after = a.world().non_send::<FrontHost>().drawn.clone();
    assert_ne!(first, after, "wheel scrolled the list");
}
