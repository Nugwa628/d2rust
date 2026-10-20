//! The front end to the game start: a saved character chosen at Nightmare
//! reaches the 0x67 with its name and the difficulty flags; a dead hardcore
//! character gets message 5304; the loading screen goes through its events
//! to the world. Synthetic saves, no game files.

use std::fs;
use std::path::PathBuf;

use d2_client::app::front_start::{registry, LoadingState, StartChoice, StartHandles};
use d2_client::bridge::world::{ActLoad, ClientWorld};
use d2_client::ui::front_end::screens::loading::Presented;
use d2_client::ui::front_end::MAIN_MENU;
use d2_client::ui::front_end::*;

fn folder(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("d2rust-fs-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn save(dir: &std::path::Path, file: &str, class: u8, status: u16) {
    let mut b = vec![0u8; 0x14F];
    b[0..4].copy_from_slice(&0xAA55_AA55u32.to_le_bytes());
    b[4..8].copy_from_slice(&0x60u32.to_le_bytes());
    b[0x24..0x26].copy_from_slice(&status.to_le_bytes());
    b[0x28] = class;
    b[0x2B] = 30;
    b[0x88] = 1;
    fs::write(dir.join(file), &b).unwrap();
}

/// A front end on `dir`, at character select.
fn at_select(dir: &std::path::Path, h: &StartHandles) -> FrontEnd {
    let mut f = FrontEnd::new(true, Box::new(true), registry(dir, h));
    f.start(false);
    assert_eq!(f.current(), MAIN_MENU);
    f.trigger(Trigger::SinglePlayer);
    assert_eq!(f.current(), CHAR_SELECT);
    f
}

#[test]
fn saved_character_at_nightmare_reaches_the_0x67() {
    let d = folder("nm");
    // Expansion, softcore, progression 5: Nightmare open, Hell off.
    save(&d, "Zed.d2s", 1, 0x0520);
    let h = StartHandles::default();
    let mut f = at_select(&d, &h);
    f.input(FrontInput::Key(13));
    f.tick();
    // The box is a pop-up of the character select (`ui-frontend-charsel-ok`).
    assert_eq!(f.current(), CHAR_SELECT);
    f.trigger(Trigger::Difficulty(1));
    let Some(Outcome::GameLoad(g)) = f.outcome() else {
        panic!("{:?}", f.outcome());
    };
    let c = StartChoice::resolve(g, &h, &d).expect("a choice");
    assert_eq!((c.name.as_str(), c.class, c.difficulty), ("Zed", 1, 1));
    assert_eq!(c.save, Some(d.join("Zed.d2s")));
    let r = c.create_request();
    assert_eq!(&r.char_name[..4], b"Zed\0");
    assert_eq!((r.class, r.difficulty), (1, 1));
    assert_eq!(r.flags, 4 | 0x10_0000);
}

#[test]
fn hardcore_flag_and_no_box_start() {
    let d = folder("hc");
    // Hardcore expansion, progression 0: no box, Normal.
    save(&d, "Hc.d2s", 3, 0x0024);
    let h = StartHandles::default();
    let mut f = at_select(&d, &h);
    f.input(FrontInput::Key(13));
    f.tick();
    let Some(Outcome::GameLoad(g)) = f.outcome() else {
        panic!("{:?}", f.outcome());
    };
    assert_eq!(g.difficulty, None);
    let c = StartChoice::resolve(g, &h, &d).unwrap();
    assert!(c.hardcore());
    assert_eq!(c.create_request().flags, 4 | 0x800 | 0x10_0000);
    assert_eq!(c.create_request().difficulty, 0);
}

#[test]
fn dead_hardcore_shows_5304_and_stays() {
    let d = folder("dead");
    save(&d, "Rip.d2s", 0, 0x002C);
    let h = StartHandles::default();
    let mut f = at_select(&d, &h);
    f.input(FrontInput::Key(13));
    f.tick();
    assert_eq!(f.outcome(), None);
    assert_eq!(f.current(), CHAR_SELECT);
    assert!(h.selection.lock().unwrap().is_none());
    let texts: Vec<_> = f
        .draw()
        .into_iter()
        .filter_map(|i| match i {
            DrawItem::Text { string_id, .. } => Some(string_id),
            _ => None,
        })
        .collect();
    assert!(texts.contains(&5304), "{texts:?}");
}

#[test]
fn loading_goes_through_its_events_to_the_world() {
    let mut s = LoadingState::default();
    assert!(s.covering());
    let mut w = ClientWorld {
        act: Some(ActLoad {
            act: 1,
            init_seed: 1,
            town_level: 40,
            f8: 0,
        }),
        ..ClientWorld::default()
    };
    assert_eq!(s.advance(&w, false), Some(Presented::Loading { frame: 1 }));
    w.in_game = true;
    // 0x04 without a placed player: still loading.
    assert!(matches!(
        s.advance(&w, false),
        Some(Presented::Loading { .. })
    ));
    assert!(s.covering());
    assert_eq!(s.advance(&w, true), Some(Presented::Black));
    assert!(!s.covering());
    assert_eq!(s.advance(&w, true), Some(Presented::World { act: 1 }));
}

#[test]
fn act_change_sequence_drives_the_states_in_spec_order() {
    use d2_client::bridge::world::SessionMark as M;
    let act = |n| ActLoad {
        act: n,
        init_seed: 1,
        town_level: 40,
        f8: 0,
    };
    let mut s = LoadingState::default();
    let mut w = ClientWorld {
        act: Some(act(0)),
        ..ClientWorld::default()
    };
    // Join: 0x03 act 0, 0x04, player placed.
    w.mark_session(M::LoadAct(0));
    w.in_game = true;
    w.mark_session(M::LoadComplete);
    assert_eq!(s.advance(&w, true), Some(Presented::Black));
    assert_eq!(s.advance(&w, true), Some(Presented::World { act: 0 }));
    assert!(!s.covering());
    // Waypoint to Act II: 0x05, 0x03 (act 1), 0x61 id 2, then 0x04.
    w.in_game = false;
    w.mark_session(M::Unload);
    w.act = Some(act(1));
    w.mark_session(M::LoadAct(1));
    w.mark_session(M::Video(2));
    // The world frame is not drawn; loading frame 0, then black after the
    // video (no redraw).
    assert_eq!(s.advance(&w, true), Some(Presented::Black));
    assert!(s.covering());
    assert_eq!(s.screen.videos, vec![2]);
    let p = &s.screen.presented;
    assert_eq!(p[p.len() - 1], Presented::Loading { frame: 0 });
    w.in_game = true;
    w.mark_session(M::LoadComplete);
    assert_eq!(s.advance(&w, true), Some(Presented::Black));
    assert!(!s.covering());
    assert_eq!(s.advance(&w, true), Some(Presented::World { act: 1 }));
}

#[test]
fn repeated_same_act_load_redraws_loading() {
    use d2_client::bridge::world::SessionMark as M;
    let a = ActLoad {
        act: 1,
        init_seed: 1,
        town_level: 40,
        f8: 0,
    };
    let mut s = LoadingState::default();
    let mut w = ClientWorld {
        act: Some(a),
        ..ClientWorld::default()
    };
    w.mark_session(M::LoadAct(1));
    s.advance(&w, false);
    // A second 0x03 of the same act leaves the model's act unchanged.
    w.mark_session(M::LoadAct(1));
    assert_eq!(s.advance(&w, false), Some(Presented::Loading { frame: 2 }));
}
