//! The credits screen's text behaviour on the user's install against shots
//! of the same screen in the game: every text glyph
//! draw (font, frame, x, y, colour k) of each shot must equal D2RUST's draw list at some scroll
//! tick. That checks the file choice and decoding, the parse into rows, blank
//! padding and colours, the wrap and the column layout and scroll
//! by the text itself, not by pixels. Ignored: needs `D2_GAME_DIR`;
//! `D2_CREDITS_SCENES` = a facts directory whose `scenes/<name>` are the shots
//! (scenes `frontend-credits-text-N`; `D2_CREDITS_PREFIX` picks them).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use d2_client::app::front_host::{add_front_end, draw_rows, FrontArt, FrontHost};
use d2_client::app::front_start::{front_host, Entry};
use d2_client::app::single_player::GameData;
use d2_client::facts::{self, DRAW_COLUMNS};
use d2_client::ui::front_end::*;
use d2_client::ui::geom::Point;

const MAX_TICKS: u32 = 1_500;

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

fn host(a: &mut App) -> Mut<'_, FrontHost> {
    a.world_mut().non_send_mut::<FrontHost>()
}

/// (font file, frame, x, y, colour k) of every text glyph, sorted.
type Glyph = (String, String, String, String, String);

/// The glyph draws of D2RUST's current frame, each with its item's colour.
fn glyphs(a: &mut App) -> BTreeSet<Glyph> {
    let mut h = host(a);
    let h = &mut *h;
    let mut items = h.front.draw();
    let art = h.art.as_mut().unwrap();
    let over = {
        let art = std::cell::RefCell::new(&mut *art);
        h.front
            .overlay(&|font, text| art.borrow_mut().text_width(font, text))
    };
    items.extend(over);
    let mut out = BTreeSet::new();
    for it in &items {
        let DrawItem::Text { color, .. } = it else {
            continue;
        };
        for r in draw_rows(std::slice::from_ref(it), art) {
            if r[1] == "CelDrawColor" {
                out.insert((
                    r[2].clone(),
                    r[4].clone(),
                    r[6].clone(),
                    r[7].clone(),
                    color.to_string(),
                ));
            }
        }
    }
    out
}

fn scene_glyphs(dir: &std::path::Path) -> BTreeSet<Glyph> {
    let t = facts::read(dir, "draws.tsv", &DRAW_COLUMNS).expect("draws.tsv");
    t.rows
        .iter()
        .filter(|r| r[1] == "CelDrawColor")
        .map(|r| {
            (
                r[2].clone(),
                r[4].clone(),
                r[6].clone(),
                r[7].clone(),
                r[14].clone(),
            )
        })
        .collect()
}

#[test]
#[ignore = "needs D2_GAME_DIR (the user's install) and D2_CREDITS_SCENES"]
fn credits_text_against_1_14d_shots() {
    let game = PathBuf::from(std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR"));
    let scenes = PathBuf::from(std::env::var("D2_CREDITS_SCENES").expect("D2_CREDITS_SCENES"))
        .join("scenes");
    let prefix = std::env::var("D2_CREDITS_PREFIX").unwrap_or_default();
    let mut names: Vec<String> = std::fs::read_dir(&scenes)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&prefix))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no scenes in {}", scenes.display());
    let want: Vec<(String, BTreeSet<Glyph>)> = names
        .iter()
        .map(|n| (n.clone(), scene_glyphs(&scenes.join(n))))
        .collect();

    let saves = std::env::temp_dir().join(format!("d2rust-credits-saves-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&saves);
    std::fs::create_dir_all(&saves).unwrap();
    let data = GameData::select(Some(&game)).expect("live data");
    let GameData::Live(d) = &data;
    let mut art = FrontArt::new(d.archives.source());
    if let Ok(t) = d2_client::app::strings::TableStrings::load(
        d.archives.as_ref(),
        d2_client::app::strings::LANG,
    ) {
        art = art.with_strings(move |id| u16::try_from(id).map(|i| t.by_id(i)).unwrap_or_default());
    }
    use d2_data::bin::TableFiles;
    let (h, _handles) = front_host(&saves, Some(art), d.archives.lod(), Entry::First);
    let mut a = app(h);
    for _ in 0..400 {
        a.update();
        if host(&mut a).front.current() == MAIN_MENU {
            break;
        }
        if host(&mut a).frames % 50 == 49 {
            host(&mut a).front.input(FrontInput::Key(0x1B));
        }
    }
    assert_eq!(host(&mut a).front.current(), MAIN_MENU, "no main menu");
    let p = Point::new(330, 516);
    host(&mut a).front.seen_pointer(p);
    a.update();
    host(&mut a).front.input(FrontInput::Down(p));
    host(&mut a).front.input(FrontInput::Up(p));

    let mut found: Vec<Option<u32>> = vec![None; want.len()];
    let mut best: Vec<(usize, u32)> = vec![(0, 0); want.len()];
    for t in 1..=MAX_TICKS {
        a.update();
        if found.iter().all(Option::is_some) {
            break;
        }
        let g = glyphs(&mut a);
        for (i, (_, w)) in want.iter().enumerate() {
            if found[i].is_some() {
                continue;
            }
            if &g == w {
                found[i] = Some(t);
            }
            let common = g.intersection(w).count();
            if common > best[i].0 {
                best[i] = (common, t);
            }
        }
    }
    let mut bad = false;
    for (i, (n, w)) in want.iter().enumerate() {
        match found[i] {
            Some(t) => println!(
                "credits-text {n}: MATCH {} glyphs at screen tick {t}",
                w.len()
            ),
            None => {
                bad = true;
                println!(
                    "credits-text {n}: DIVERGED best tick {} equal {} of {} glyphs",
                    best[i].1,
                    best[i].0,
                    w.len()
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&saves);
    assert!(!bad, "a shot diverged");
}
