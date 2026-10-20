use super::*;

fn fresh() -> Cursor {
    Cursor::init(0, 800, 600, 1000)
}

#[test]
fn type_table() {
    assert_eq!(TYPES.len(), 7);
    let names: Vec<&str> = TYPES.iter().map(|t| t.name).collect();
    assert_eq!(
        names,
        ["Gaunt", "grasp", "ohand", "orotate", "ppress", "protate", "buysell"]
    );
    let t = |i: usize| {
        let c = &TYPES[i];
        (c.animated, c.loops, c.frames, c.step, c.press, c.shop_draw)
    };
    assert_eq!(t(0), (false, false, 1, 0, true, false));
    assert_eq!(t(1), (true, false, 8, 0, true, false));
    assert_eq!(t(2), (true, false, 8, 0x40, true, false));
    assert_eq!(t(3), (true, true, 8, 0x20, true, false));
    assert_eq!(t(4), (true, true, 8, 0x40, true, false));
    assert_eq!(t(5), (true, true, 8, 0x40, true, false));
    assert_eq!(t(6), (false, false, 1, 0, false, true));
    assert_eq!(cel_path(2), "data\\global\\ui\\cursor\\ohand");
}

#[test]
fn init_state() {
    let c = fresh();
    assert_eq!((c.drawn, c.s, c.t, c.f, c.idle), (true, 1, 5, 0, 1000));
    assert_eq!((c.mx, c.my), (400, 300));
    assert!(!c.item);
}

#[test]
fn mouse_move() {
    let mut c = fresh();
    c.s = 4;
    assert_eq!(
        c.mouse_move(10, 20, 2000, 800, 600, false),
        MoveResult::Done
    );
    assert_eq!((c.mx, c.my, c.idle), (10, 20, 2000));
    // s 4 or 2 → s 3, ohand, the last frame run backwards
    assert_eq!((c.s, c.t, c.f), (3, 2, 7 * 256));
    let mut c = fresh();
    c.s = 2;
    c.mouse_move(1, 1, 5, 800, 600, false);
    assert_eq!((c.s, c.t, c.f), (3, 2, 1792));
    // other states unchanged
    let mut c = fresh();
    c.mouse_move(1, 1, 5, 800, 600, false);
    assert_eq!((c.s, c.t), (1, 5));
    // outside the window with the clip on: clamped, handler returns (idle kept)
    let mut c = fresh();
    c.s = 4;
    assert_eq!(
        c.mouse_move(900, -3, 7, 800, 600, true),
        MoveResult::Clamped { x: 799, y: 0 }
    );
    assert_eq!((c.idle, c.s), (1000, 4));
    // inside with the clip on: the normal path
    assert_eq!(c.mouse_move(5, 5, 7, 800, 600, true), MoveResult::Done);
    assert_eq!(c.idle, 7);
}

#[test]
fn button_down_and_up() {
    let mut c = fresh();
    c.button_down(30, 40, 2000);
    assert_eq!((c.mx, c.my, c.idle), (30, 40, 2000));
    assert_eq!((c.s, c.t, c.f), (5, 4, 0));
    // a second down while pressed does nothing
    c.f = 77;
    c.button_down(1, 1, 3000);
    assert_eq!(c.f, 77);
    c.button_up(5, 6, 4000);
    assert_eq!((c.s, c.t, c.f, c.mx, c.idle), (1, 5, 0, 5, 4000));
    // up without a press: positions only
    let mut c = fresh();
    c.t = 2;
    c.button_up(1, 2, 9);
    assert_eq!((c.s, c.t, c.mx, c.idle), (1, 2, 1, 9));
    // the shop type has no press animation
    let mut c = fresh();
    c.shop_empty(3, false);
    c.button_down(0, 0, 1);
    assert_eq!((c.s, c.t), (7, 6));
}

#[test]
fn cursor_item_changes() {
    let mut c = fresh();
    c.f = 99;
    c.set_item(true);
    assert_eq!((c.s, c.t, c.f, c.item), (4, 5, 0, true));
    c.set_item(false);
    assert_eq!((c.s, c.t, c.f, c.item), (1, 5, 0, false));
    c.shop_item(3);
    assert_eq!((c.s, c.t, c.f, c.item), (6, 6, 3, true));
    c.shop_empty(2, false);
    assert_eq!((c.s, c.t, c.f, c.item), (7, 6, 2, false));
    c.shop_empty(2, true);
    assert_eq!(c.s, 8);
}

#[test]
fn step_machine() {
    let mut seed = 0u64;
    // state 3 runs backwards; a looping type wraps, another returns to protate
    let mut c = fresh();
    c.s = 3;
    c.t = 2;
    c.f = 7 * 256;
    c.step(2000, true, &mut seed).unwrap();
    assert_eq!((c.f, c.last), (7 * 256 - 0x40, 2000));
    // within 16 ms: nothing
    c.step(2016, true, &mut seed).unwrap();
    assert_eq!(c.f, 7 * 256 - 0x40);
    c.f = 0x20;
    c.step(2100, true, &mut seed).unwrap();
    assert_eq!((c.s, c.t, c.f), (1, 5, 0)); // ohand does not loop
    let mut c = fresh();
    c.s = 3;
    c.t = 3; // orotate loops
    c.f = 0x10;
    c.step(2000, true, &mut seed).unwrap();
    assert_eq!((c.s, c.f), (3, 0x10 - 0x40 + 8 * 256));
    // state 1: the seed steps and f += 0x20 when bits 0-5 are below 16
    let mut c = fresh();
    let mut seed = 0u64;
    seed = seed_step(seed.max(1));
    let probe = seed_step(seed);
    let expect = if (probe as u32) & 0x3F < 16 { 0x20 } else { 0 };
    c.step(2000, true, &mut seed).unwrap();
    assert_eq!(seed, probe);
    assert_eq!(c.f, expect);
    // without a local player nothing advances
    let mut c = fresh();
    let mut seed = 5u64;
    c.step(2000, false, &mut seed).unwrap();
    assert_eq!((c.f, seed), (0, 5));
    // other states: f += step; ohand (state 2) finishing → s 4, orotate
    let mut c = fresh();
    c.s = 2;
    c.t = 2;
    c.f = 7 * 256 + 0xC0;
    c.step(2000, true, &mut seed).unwrap();
    assert_eq!((c.s, c.t, c.f), (4, 3, 0));
    // ppress (state 5) loops through; at the end of a non-looping press type → state 1
    let mut c = fresh();
    c.s = 5;
    c.t = 4;
    c.f = 8 * 256 - 0x20;
    c.step(2000, true, &mut seed).unwrap();
    assert_eq!(c.f, 0x20);
    // grasp (animated, no loop) in an unknown state past its end: fatal
    let mut c = fresh();
    c.s = 4;
    c.t = 1;
    c.f = 8 * 256;
    assert_eq!(c.step(2000, true, &mut seed), Err(CursorFatal(4)));
    // idle for more than 5 s in state 1 → ohand
    let mut c = fresh();
    c.t = 0; // not animated: only the idle test
    c.step(1000 + 5000, true, &mut seed).unwrap();
    assert_eq!((c.s, c.t), (1, 0));
    c.step(1000 + 5001, true, &mut seed).unwrap();
    assert_eq!((c.s, c.t, c.f), (2, 2, 0));
}

#[test]
fn draw() {
    let mut seed = 0u64;
    let mut c = Cursor::init(0, 800, 600, 0);
    c.mx = 100;
    c.my = 200;
    c.f = 3 * 256 + 5;
    // types 0-5: frame f >> 8 at the clamped mouse position, then the step
    let d = c.draw(800, 600, None, 100, true, &mut seed).unwrap();
    assert_eq!(
        d,
        Some(CursorDraw::Cel {
            t: 5,
            frame: 3,
            x: 100,
            y: 200
        })
    );
    assert_ne!(c.last, 0, "the step ran after the draw");
    // clamping with an x adjust
    let mut c = Cursor::init(4, 800, 600, 0);
    c.mx = -50;
    c.my = 999;
    let d = c.draw(800, 600, None, 1, false, &mut seed).unwrap();
    assert_eq!(
        d,
        Some(CursorDraw::Cel {
            t: 5,
            frame: 0,
            x: 4,
            y: 599
        })
    );
    c.mx = 5000;
    c.my = -5;
    let d = c.draw(800, 600, None, 1, false, &mut seed).unwrap();
    assert_eq!(
        d,
        Some(CursorDraw::Cel {
            t: 5,
            frame: 0,
            x: 800 - 4 - 1,
            y: 0
        })
    );
    // an item unit on the cursor: its graphic centred, halves rounded down;
    // no step runs
    let mut c = Cursor::init(2, 800, 600, 0);
    c.mx = 100;
    c.my = 100;
    c.set_item(true);
    let before = c.clone();
    let d = c
        .draw(800, 600, Some((57, 85)), 99999, true, &mut seed)
        .unwrap();
    assert_eq!(
        d,
        Some(CursorDraw::Item {
            x: 2 + 100 - 28,
            y: 100 - 42
        })
    );
    assert_eq!(c, before);
    // type 6: frame f unshifted at (adj + mx, my + 33), no clamp, no step
    let mut c = Cursor::init(1, 800, 600, 0);
    c.mx = -7;
    c.my = 9;
    c.shop_item(4);
    let d = c.draw(800, 600, None, 99999, true, &mut seed).unwrap();
    assert_eq!(
        d,
        Some(CursorDraw::Cel {
            t: 6,
            frame: 4,
            x: -6,
            y: 42
        })
    );
    // with a shop cursor state (6 or above) an item graphic is not drawn
    let d = c
        .draw(800, 600, Some((10, 10)), 5, true, &mut seed)
        .unwrap();
    assert!(matches!(d, Some(CursorDraw::Cel { t: 6, .. })));
    // nothing while not drawn
    c.drawn = false;
    assert_eq!(c.draw(800, 600, None, 5, true, &mut seed).unwrap(), None);
}

// Story check of the timeline (the claim of r13 itself is exempt).
#[test]
fn untouched_mouse_story() {
    // protate advances by chance for 5 s, then ohand once (32 steps), then
    // orotate loops; a move during orotate plays ohand backwards to protate
    let mut c = Cursor::init(0, 800, 600, 0);
    let mut seed = 1u64;
    let now = std::cell::Cell::new(0u32);
    let tick = |c: &mut Cursor, seed: &mut u64| {
        now.set(now.get() + 17);
        c.draw(800, 600, None, now.get(), true, seed).unwrap();
    };
    while c.s == 1 {
        tick(&mut c, &mut seed);
        assert!(now.get() < 6000, "idle ends after 5 s");
    }
    assert!(now.get() > 5000);
    assert_eq!((c.s, c.t, c.f), (2, 2, 0));
    let mut steps = 0;
    while c.s == 2 {
        tick(&mut c, &mut seed);
        steps += 1;
    }
    // ohand: 8 frames × 256 / 0x40 = 32 steps
    assert_eq!(steps, 32);
    assert_eq!((c.s, c.t), (4, 3));
    // orotate loops every 64 steps (step 0x20, 8 frames)
    for _ in 0..70 {
        tick(&mut c, &mut seed);
    }
    assert_eq!((c.s, c.t), (4, 3));
    // a move plays ohand backwards (29 steps from 0x700) back to protate
    c.mouse_move(3, 3, now.get(), 800, 600, false);
    assert_eq!((c.s, c.t, c.f), (3, 2, 0x700));
    let mut back = 0;
    while c.s == 3 {
        tick(&mut c, &mut seed);
        back += 1;
    }
    assert_eq!(back, 29);
    assert_eq!((c.s, c.t, c.f), (1, 5, 0));
    // a press shows ppress until the release
    c.button_down(0, 0, now.get());
    assert_eq!((c.s, c.t), (5, 4));
    c.button_up(0, 0, now.get());
    assert_eq!((c.s, c.t), (1, 5));
}

#[test]
fn messages_that_reach_the_cursor() {
    let mut c = fresh();
    // WM_NCMOUSEMOVE: not drawn; the OS cursor call only for HTCAPTION
    assert!(!c.nc_mouse_move(1));
    assert!(!c.drawn);
    assert!(c.nc_mouse_move(2));
    // the next move draws again
    c.mouse_move(1, 1, 5, 800, 600, false);
    assert!(c.drawn);
    // a world click does not touch the cursor type: only panel presses
    // call button_down, so a plain cursor stays protate
    assert_eq!((c.s, c.t), (1, 5));
    // the up transition twice is harmless
    c.button_down(0, 0, 1);
    c.button_up(0, 0, 2);
    c.button_up(0, 0, 3);
    assert_eq!((c.s, c.t), (1, 5));
}

#[test]
fn place_owes_the_step_that_draw_runs() {
    let mut a = fresh();
    let mut b = a.clone();
    let (d, owed) = a.place(640, 480, None);
    assert!(owed, "a type-cel draw is followed by the step");
    let mut seed = 0x0000_029A_0000_0001u64;
    let drawn = b.draw(640, 480, None, 100, true, &mut seed).unwrap();
    assert_eq!(d, drawn);
    let mut seed2 = 0x0000_029A_0000_0001u64;
    a.step(100, true, &mut seed2).unwrap();
    assert_eq!((a, seed2), (b, seed));
    // An item on the cursor: no step owed.
    let c = fresh();
    assert!(!c.place(640, 480, Some((28, 28))).1);
}

/// A host `WM_MOUSEMOVE` replayed from a check run: idle start:= its clock; an
/// idle `ohand` runs back, `protate` keeps its state.
#[test]
fn a_host_move_resets_the_idle_start_in_place() {
    let mut c = fresh();
    c.s = 2;
    c.host_move(7000);
    assert_eq!((c.s, c.t, c.f, c.idle), (3, 2, 0x700, 7000));
    c.set_item(false);
    c.host_move(7100);
    assert_eq!((c.s, c.t, c.f, c.idle), (1, 5, 0, 7100));
}

/// A cursor jump keeps the jump point drawn until
/// the host delivers its `WM_MOUSEMOVE`, which reports y − 3
/// (`fx-charged-bolt-cast`: (790, 262) at tick 137, (790, 259) from 138).
#[test]
fn a_jump_moves_at_its_host_message() {
    let mut c = fresh();
    c.jump_to(790, 262, (790, 259));
    assert_eq!((c.mx, c.my, c.idle), (790, 262, 1000));
    c.host_move(5480);
    assert_eq!((c.mx, c.my, c.idle, c.host_at), (790, 259, 5480, None));
    c.host_move(5500);
    assert_eq!((c.mx, c.my), (790, 259));
}

/// Seen from the UI box of the 16:9 frame (columns −134 … 933), the cursor
/// still reaches both frame edges; `place` is `place_within` from 0.
#[test]
fn the_cursor_reaches_the_wide_frame_edges() {
    let mut c = fresh();
    c.mouse_move(-130, 300, 1, 800, 600, false);
    let (d, _) = c.place_within(-134, 1068, 600, None);
    let Some(CursorDraw::Cel { x, .. }) = d else {
        panic!("{d:?}")
    };
    assert_eq!(x, -130 + c.adj);
    c.mouse_move(5_000, 300, 2, 800, 600, false);
    let (d, _) = c.place_within(-134, 1068, 600, None);
    let Some(CursorDraw::Cel { x, .. }) = d else {
        panic!("{d:?}")
    };
    assert_eq!(x, -134 + 1068 - c.adj - 1);
    assert_eq!(c.place(800, 600, None), c.place_within(0, 800, 600, None));
}
