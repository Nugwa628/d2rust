//! The dead / no-player close of [`OriginalUi::cube_poll`] on a synthetic model.

use super::*;
use crate::bridge::items::mode;
use crate::bridge::world::ClientWorld;
use crate::ui::geom::Point;
use crate::ui::layout::Screen;
use crate::ui::original::UiConfig;
use crate::ui::panel::{UiCtx, UiEvent};
use crate::ui::panels::inv_items::tests::world;
use crate::ui::NoPanelRules;

fn ui() -> (OriginalUi, UiRoot) {
    let config = UiConfig {
        screen: Screen::R800,
        expansion_installed: true,
    };
    let ui = OriginalUi::new(config, None).unwrap();
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    ui.install(&mut root).unwrap();
    (ui, root)
}

// A missing cube does not close the panel; a dead local player (mode
// 0x11) does, with 0x4F 0x17 twice. (Before this audit the code closed on
// the cube's absence; the only tests are the exit flag and the
// dead-player check.)
#[test]
fn the_cube_panel_closes_on_death_not_when_the_cube_leaves() {
    let (mut u, mut root) = ui();
    let with_cube = world(&[(7, mode::STORED, (0, 0, 0, 1), b"box ")], None);
    u.set_ui(u32::from(id::CUBE), 0, false).unwrap();
    u.sync_root(&mut root);
    u.cube_poll(&with_cube, &mut root).unwrap();
    assert!(u.is_open(id::CUBE));
    assert!(root.intents().is_empty());
    // The cube moved out: the panel stays, nothing is sent.
    let without = world(&[(8, mode::STORED, (0, 2, 0, 1), b"hp1 ")], None);
    u.cube_poll(&without, &mut root).unwrap();
    assert!(u.is_open(id::CUBE));
    assert!(root.intents().is_empty());
    // The player died: the panel closes and 0x4F 0x17 leaves twice.
    let mut dead = without.clone();
    dead.units
        .get_mut(&dead.local_player.unwrap())
        .unwrap()
        .mode = 0x11;
    u.cube_poll(&dead, &mut root).unwrap();
    assert!(!u.is_open(id::CUBE));
    let sent: Vec<Vec<u8>> = root.take_intents().into_iter().map(|i| i.0).collect();
    assert_eq!(sent.len(), 2);
    assert!(sent.iter().all(|b| b[..2] == [0x4F, 0x17]), "{sent:?}");
    // Closed: nothing more on the next pass.
    u.cube_poll(&dead, &mut root).unwrap();
    assert!(root.intents().is_empty());
}

struct Strs;
impl crate::ui::panel::StringLookup for Strs {
    fn get(&self, _: &str) -> Option<&[u16]> {
        None
    }
    fn get_id(&self, id: u16) -> Option<&[u16]> {
        const CLOSE: [u16; 5] = [
            b'C' as u16,
            b'l' as u16,
            b'o' as u16,
            b's' as u16,
            b'e' as u16,
        ];
        const TRANSMUTE: [u16; 3] = [b'T' as u16, b'r' as u16, b'x' as u16];
        match id {
            4144 => Some(&CLOSE),
            3341 => Some(&TRANSMUTE),
            _ => None,
        }
    }
}

fn tips(u: &mut OriginalUi, root: &UiRoot, w: &ClientWorld, at: Point) -> Vec<String> {
    use crate::ui::draw::UiDraw;
    let e = UiEvent::CursorMoved(at);
    u.before_event(e, w);
    let ctx = UiCtx {
        tick: 0,
        world: w,
        strings: &Strs,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    root.draw(&ctx, &mut out);
    out.iter()
        .filter_map(|d| match d {
            UiDraw::Text(t) => Some(String::from_utf16_lossy(&t.text)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_cube_buttons_show_their_tool_tips_on_hover() {
    let (mut u, mut root) = ui();
    let w = world(&[(7, mode::STORED, (0, 0, 0, 1), b"box ")], None);
    u.set_ui(u32::from(id::CUBE), 0, false).unwrap();
    u.sync_root(&mut root);
    let s = Screen::R800;
    let close = Point::new(s.sx() + 290, s.h + s.sy() - 80);
    let trans = Point::new(s.sx() + 160, s.h + s.sy() - 200);
    assert_eq!(tips(&mut u, &root, &w, close), ["Close"]);
    assert_eq!(tips(&mut u, &root, &w, trans), ["Trx"]);
    assert!(tips(&mut u, &root, &w, Point::new(5, 5)).is_empty());
}

/// A cube in the stash page and a living local player (mode 1): the
/// message path's gate reads the player's life.
fn alive_world() -> ClientWorld {
    let mut w = world(&[(7, mode::STORED, (0, 0, 0, 1), b"box ")], None);
    let p = w.local_player.unwrap();
    w.units.get_mut(&p).unwrap().mode = 1;
    w
}

fn click(u: &mut OriginalUi, root: &mut UiRoot, w: &ClientWorld, at: Point) {
    use crate::ui::PointerButton;
    let ctx = UiCtx {
        tick: 0,
        world: w,
        strings: &Strs,
    };
    for e in [
        UiEvent::Press {
            button: PointerButton::Left,
            at,
        },
        UiEvent::Release {
            button: PointerButton::Left,
            at,
        },
    ] {
        u.before_event(e, w);
        let routed = root.dispatch(e, &ctx);
        u.after_event(root, e, routed).unwrap();
    }
}

fn open_cube(u: &mut OriginalUi, root: &mut UiRoot, w: &ClientWorld) {
    use crate::bridge::output::Output;
    u.apply_output(
        &Output::TradeAction {
            code: 0x15,
            dead_or_absent: false,
        },
        w,
    )
    .unwrap();
    u.sync_root(root);
}

#[test]
fn opening_and_closing_the_cube_twice_sends_0x17_each_time() {
    let (mut u, mut root) = ui();
    let w = alive_world();
    let s = Screen::R800;
    let close = Point::new(s.sx() + 290, s.h + s.sy() - 80);
    for _ in 0..2 {
        open_cube(&mut u, &mut root, &w);
        assert!(u.is_open(id::CUBE));
        click(&mut u, &mut root, &w, close);
        assert!(!u.is_open(id::CUBE));
        let sent: Vec<Vec<u8>> = root.take_intents().into_iter().map(|i| i.0).collect();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0][..2], [0x4F, 0x17]);
    }
}

fn frames(u: &OriginalUi, root: &UiRoot, w: &ClientWorld, tick: u64) -> Vec<u32> {
    use crate::ui::draw::UiDraw;
    let ctx = UiCtx {
        tick,
        world: w,
        strings: &Strs,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    root.draw(&ctx, &mut out);
    let horadric = u.shared.borrow().tables.files.id("menu\\horadric").unwrap();
    out.iter()
        .filter_map(|d| match d {
            UiDraw::Image(i) if i.image.file == horadric => {
                // Draw mode 3, no remap.
                assert_eq!((i.look.mode, i.look.remap), (3, crate::ui::Remap::None));
                Some(i.image.frame)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn the_transmute_animation_plays_frames_0_to_29_on_the_70ms_steps() {
    use crate::bridge::output::Output;
    let (mut u, mut root) = ui();
    let w = alive_world();
    open_cube(&mut u, &mut root, &w);
    assert!(frames(&u, &root, &w, 100).is_empty());
    let s = Screen::R800;
    let trans = Point::new(s.sx() + 160, s.h + s.sy() - 200);
    // The transmute button starts nothing (a gem transmute shows none).
    click(&mut u, &mut root, &w, trans);
    assert_eq!(root.take_intents().len(), 1);
    assert!(frames(&u, &root, &w, 101).is_empty());
    // An `hst ` arrives in page 3: the client quest
    // record lacks 10.11, so it is set and the animation starts at the
    // next draw (tick 0, stamp 0 ms).
    u.apply_output(&Output::HoradricItem { code: *b"hst " }, &w)
        .unwrap();
    // 40 ms frames: a step needs more than 70 ms, so every second frame.
    let mut seen = Vec::new();
    for t in 0..80u64 {
        let f = frames(&u, &root, &w, t);
        if let Some(&n) = f.first() {
            if seen.last() != Some(&n) {
                seen.push(n);
            }
        }
    }
    assert_eq!(seen, (0..30).collect::<Vec<u32>>());
    // Stopped after frame 29: nothing drawn (frame 30 never).
    assert!(frames(&u, &root, &w, 200).is_empty());
    // 10.11 is now set in the client copy: a second `hst ` plays nothing.
    u.apply_output(&Output::HoradricItem { code: *b"hst " }, &w)
        .unwrap();
    assert!(frames(&u, &root, &w, 300).is_empty());
}

// The > 70 ms steps are wall time: at 144 draws a second 40 ms per draw
// stepped every second draw and played the 30 steps in 0.4 s.
#[test]
fn the_transmute_animation_lasts_the_same_at_25_60_and_144_draws_a_second() {
    use crate::bridge::output::Output;
    use crate::ui::ui_clock::{draw_ms, period_ms, RATES};
    for fps in RATES {
        let (mut u, mut root) = ui();
        let w = alive_world();
        open_cube(&mut u, &mut root, &w);
        u.set_menu_wall_ms(Some(draw_ms(fps, 0)));
        u.apply_output(&Output::HoradricItem { code: *b"hst " }, &w)
            .unwrap();
        let (mut i, mut seen) = (0, Vec::new());
        loop {
            u.set_menu_wall_ms(Some(draw_ms(fps, i)));
            let Some(&n) = frames(&u, &root, &w, i).first() else {
                break;
            };
            if seen.last() != Some(&n) {
                seen.push(n);
            }
            i += 1;
            assert!(i < 100_000);
        }
        assert_eq!(seen, (0..30).collect::<Vec<u32>>(), "{fps}/s");
        // 30 steps, each at the first draw more than 70 ms after the last.
        let ms = draw_ms(fps, i) - draw_ms(fps, 0);
        let (lo, hi) = (30 * 71, 30 * (70 + period_ms(fps)));
        assert!((lo..=hi).contains(&ms), "{fps}/s: {ms} ms");
    }
}
