//! The mouse cursor in play: the cursor machine ([`Cursor`]) fed with the UI's
//! pointer events (move, button down, button up, leaving the window) and
//! the cursor item, drawn once per frame after the whole UI pass by the top
//! adapter (the item's graphic; else the type's cel; the step runs after the
//! cel draw). The OS cursor is hidden by the play window (`app::play`).
// D2RUST's own, unverified (M22):
// - the clock (`GetTickCount`) is the client frame count at 40 ms;
// - the step's seed draws use a UI copy of the local player's client seed
//   (`ClientUnit::seed`), taken when the cursor first sees it; the draws are
//   not written back to the model (the game's draws would change the seed);
// - the shop cursors (type 6) are not driven
//   (their callers are the shop mouse handlers);
// - the mouse-move clamp is off (no `SetCursorPos` in the D2RUST
//   window).

use super::Shared;
use crate::bridge::items;
use crate::bridge::world::ClientWorld;
use crate::ui::cursor::{CursorDraw, TYPES};
use crate::ui::draw::{CelLook, ImageRef, ImageRequest, UiDraw, UiDrawSink};
use crate::ui::geom::Point;
use crate::ui::panel::{PointerButton, UiCtx, UiEvent};

use super::OriginalUi;

/// The cel names the art loader resolves (`cursor\<name>`).
pub(super) fn cursor_files() -> Vec<String> {
    TYPES
        .iter()
        .map(|t| format!("cursor\\{}", t.name))
        .collect()
}

/// The client clock of the cursor (module doc). The cursor's `GetTickCount`:
/// the host clock the app sets ([`OriginalUi::set_host_now`]: a check run's
/// recorded frame clock), else the UI clock ([`super::ui_clock`]: the wall
/// clock in live play, so the 16 ms step gate and the 5 s idle hold at any
/// frame rate; 40 ms per bridge frame otherwise).
fn now(sh: &Shared, world: &ClientWorld) -> u32 {
    sh.host_now
        .get()
        .unwrap_or_else(|| sh.ui_ms32(world.frames))
}

/// The cursor press as the panels call it: only the character
/// panel's add buttons, the control panel's new-stats / new-skills buttons and
/// the skill tree's icons press the cursor; no window message does.
pub(super) fn cursor_press(sh: &Shared, world: &ClientWorld, at: Point) {
    let t = now(sh, world);
    sh.cursor.borrow_mut().button_down(at.x, at.y, t);
}

impl OriginalUi {
    /// The owed cursor step of the last cursor draw, on the
    /// client seed shared with the weather and sound draws: run once per
    /// drawn frame, after the frame's weather update. Nothing owed, or no
    /// linked seed: nothing.
    pub fn cursor_step(
        &mut self,
        world: &ClientWorld,
    ) -> Result<(), crate::ui::cursor::CursorFatal> {
        let sh = self.shared.borrow();
        if !sh.cursor_step_owed.replace(false) {
            return Ok(());
        }
        let Some(shared) = sh.client_seed.as_ref() else {
            return Ok(());
        };
        let mut held = shared.lock().unwrap_or_else(|e| e.into_inner());
        held.sync(world);
        let mut seed = held
            .seed()
            .map_or(0, |s| u64::from(s.lo) | (u64::from(s.hi) << 32));
        let player = world.local().is_some();
        let r = sh
            .cursor
            .borrow_mut()
            .step(now(&sh, world), player, &mut seed);
        if let Some(s) = held.seed() {
            *s = d2_sim::rng::Seed::new(seed as u32, (seed >> 32) as u32);
        }
        r
    }

    /// A tick whose draw waits for the GPU steps the cursor in its first pass
    /// before the pass that draws it: that
    /// draw shows the cursor as the step found it, the frame drawn before
    /// stepping (the step follows the draw). Keeps the type and frame for the
    /// later draws of `tick`.
    pub fn hold_cursor_frame(&mut self, tick: u64) {
        let sh = self.shared.borrow();
        let c = sh.cursor.borrow();
        sh.cursor_hold.set(Some((tick, c.t, c.f)));
    }

    /// Owes a cursor step as a drawn cursor does (`cursor_step` then runs
    /// it): a recorded frame whose UI pass D2RUST does not repeat.
    pub fn owe_cursor_step(&mut self) {
        self.shared.borrow().cursor_step_owed.set(true);
    }

    /// Owes a step that passes the 16 ms gate at the clock `now`: an extra draw
    /// of a tick (one forced step each). The pinned clock writes last:=
    /// clock − 40 at each frame start, so every draw steps, paused draws
    /// included, all on the same virtual clock.
    pub fn force_cursor_step(&mut self, now: u32) {
        let sh = self.shared.borrow();
        sh.host_now.set(Some(now));
        sh.cursor.borrow_mut().last = now.wrapping_sub(40);
        sh.cursor_step_owed.set(true);
    }

    /// The host clock the cursor reads (`GetTickCount`): a check run replays
    /// the recorded clock of each drawn frame; `None`: 40 ms per bridge frame.
    pub fn set_host_now(&mut self, now: Option<u32>) {
        self.shared.borrow().host_now.set(now);
    }

    /// The UI's wall clock in live play: the host's wall clock in ms
    /// (`GetTickCount`), which runs on while the single-player pause stops the
    /// server tick. The Esc menu's pentagrams and the UI timers
    /// ([`super::ui_clock`]) read it. `None` (check runs): the pentagrams
    /// step on the server tick's 40 ms (the pinned clock), the timers on
    /// 40 ms per bridge frame.
    pub fn set_menu_wall_ms(&mut self, now: Option<u64>) {
        let mut sh = self.shared.borrow_mut();
        sh.menu_wall_ms.set(now);
        if let Some(c) = sh.esc.controls.as_mut() {
            c.set_wall_ms(now);
        }
    }

    /// The recorded cursor timers at the first drawn frame of a check run
    /// (`last_step`, `idle_since`).
    pub fn set_cursor_timers(&mut self, last: u32, idle: u32) {
        let sh = self.shared.borrow();
        let mut c = sh.cursor.borrow_mut();
        c.last = last;
        c.idle = idle;
    }

    /// A host window message the cursor saw before this frame (a check run's
    /// recorded `idle_since`, later than the cursor's own): the move handler at
    /// the pointer's position at that clock (only the window handlers write the
    /// idle start). Nothing when the cursor's own idle start is already as
    /// late.
    pub fn host_cursor_message(&mut self, idle: u32) {
        let sh = self.shared.borrow();
        let mut c = sh.cursor.borrow_mut();
        if idle > c.idle {
            c.host_move(idle);
        }
    }

    /// The pointer events the cursor's window handlers see: the
    /// move, the left button up (neither is consumed) and the pointer
    /// leaving the window (`WM_NCMOUSEMOVE`: not drawn). The button down is
    /// not one of them: only some panel presses run it ([`cursor_press`]).
    pub(super) fn cursor_event(&mut self, e: UiEvent, world: &ClientWorld) {
        let sh = self.shared.borrow();
        let (w, h) = (sh.config.screen.w, sh.config.screen.h);
        let mut c = sh.cursor.borrow_mut();
        let t = now(&sh, world);
        match e {
            UiEvent::CursorMoved(p) => {
                c.mouse_move(p.x, p.y, t, w, h, false);
            }
            UiEvent::Release {
                button: PointerButton::Left,
                at,
            } => c.button_up(at.x, at.y, t),
            UiEvent::CursorLeft => {
                c.nc_mouse_move(0);
            }
            _ => {}
        }
    }
}

/// The cursor draw, after every other UI draw.
pub(super) fn draw(sh: &Shared, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
    let mut c = sh.cursor.borrow_mut();
    sh.cursor_item_drawn.set(false);
    // r7: every change of the cursor item.
    let item = items::cursor_item(ctx.world);
    if item.is_some() != c.item {
        c.set_item(item.is_some());
    }
    let me = ctx.world.local();
    let gfx = item
        .as_ref()
        .and_then(|it| sh.items.cursor_graphic_size(&sh.tables.files, it));
    let h = sh.config.screen.h;
    let frame_clip = sh.config.screen.frame_rect();
    let (left, w) = (frame_clip.x, i32::from(frame_clip.w));
    // With the shared client seed linked,
    // the step is owed and runs after the frame's weather update
    // ([`OriginalUi::cursor_step`]); unlinked, it runs here on a local copy.
    let d = if sh.client_seed.is_some() {
        let held = sh
            .cursor_hold
            .get()
            .filter(|&(tick, ..)| tick == ctx.world.server_ticks);
        let (d, step) = match held {
            Some((_, t, f)) => {
                let mut shown = c.clone();
                shown.t = t;
                shown.f = f;
                shown.place_within(left, w, h, gfx)
            }
            None => c.place_within(left, w, h, gfx),
        };
        if step {
            sh.cursor_step_owed.set(true);
        }
        Ok(d)
    } else {
        let mut seed = sh.cursor_seed.get().unwrap_or_else(|| {
            me.and_then(|u| u.seed)
                .map_or(0, |(lo, hi)| u64::from(lo) | (u64::from(hi) << 32))
        });
        let (d, step) = c.place_within(left, w, h, gfx);
        let d = if step {
            c.step(now(sh, ctx.world), me.is_some(), &mut seed)
                .map(|_| d)
        } else {
            Ok(d)
        };
        if me.is_some() {
            sh.cursor_seed.set(Some(seed));
        }
        d
    };
    match d {
        Ok(Some(CursorDraw::Item { .. })) => {
            // The item's graphic, centred on the mouse (`inv_items`); at
            // 16:9 its clip widens from the UI box to the frame.
            let mut item = Vec::new();
            let drawn =
                sh.items
                    .draw_cursor(ctx.world, &sh.tables.files, (29, 29), sh.mouse, &mut item);
            sh.cursor_item_drawn.set(drawn);
            for mut d in item {
                if let UiDraw::Image(i) = &mut d {
                    i.clip = frame_clip;
                }
                out.push(d);
            }
        }
        Ok(Some(CursorDraw::Cel { t, frame, x, y })) => {
            let name = format!("cursor\\{}", TYPES[usize::from(t)].name);
            if let Some(file) = sh.tables.files.id(&name) {
                out.push(UiDraw::Image(ImageRequest {
                    image: ImageRef { file, frame },
                    at: Point::new(x, y),
                    clip: frame_clip,
                    look: CelLook::PLAIN,
                    call: crate::ui::draw::CelCall::Draw,
                }));
            }
        }
        // Not drawn, or a fatal state of the step: nothing.
        Ok(None) | Err(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use crate::bridge::world::{ClientUnit, ClientWorld, UnitKey, PLAYER};
    use crate::ui::draw::UiDraw;
    use crate::ui::layout::Screen;
    use crate::ui::original::{OriginalUi, UiConfig};
    use crate::ui::panel::{NoStrings, UiCtx, UiEvent};
    use crate::ui::{NoPanelRules, Point, PointerButton, UiRoot};

    fn setup() -> (OriginalUi, UiRoot, ClientWorld) {
        let config = UiConfig {
            screen: Screen::R800,
            expansion_installed: true,
        };
        let ui = OriginalUi::new(config, None).unwrap();
        let mut root = UiRoot::new(Box::new(NoPanelRules));
        ui.install(&mut root).unwrap();
        let mut w = ClientWorld::default();
        let me = UnitKey::new(PLAYER, 1);
        let mut p = ClientUnit::new(me);
        p.mode = 1;
        p.position = Some((1000, 1000));
        p.seed = Some((1, 2));
        w.units.insert(me, p);
        w.local_player = Some(me);
        (ui, root, w)
    }

    fn send(ui: &mut OriginalUi, root: &mut UiRoot, w: &ClientWorld, e: UiEvent) {
        let ctx = UiCtx {
            tick: w.frames,
            world: w,
            strings: &NoStrings,
        };
        ui.before_event(e, w);
        let r = root.dispatch(e, &ctx);
        ui.after_event(root, e, r).unwrap();
    }

    /// The last draw of the frame: (file name, frame, x, y).
    fn last(ui: &OriginalUi, root: &UiRoot, w: &ClientWorld) -> (String, u32, i32, i32) {
        let ctx = UiCtx {
            tick: w.frames,
            world: w,
            strings: &NoStrings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        root.draw(&ctx, &mut out);
        let files = ui.files();
        match out.last() {
            Some(UiDraw::Image(i)) => (
                files.name(i.image.file).unwrap().to_string(),
                i.image.frame,
                i.at.x,
                i.at.y,
            ),
            d => panic!("the cursor is the last draw: {d:?}"),
        }
    }

    // The play UI draws the cursor last: protate (t 5) at the mouse
    // after a move, ppress (t 4) while a button is down, protate again on
    // the release, and ohand (t 2) after 5 s without input.
    #[test]
    fn the_cursor_cel_follows_the_pointer_and_the_buttons() {
        let (mut ui, mut root, mut w) = setup();
        let at = Point::new(400, 300);
        send(&mut ui, &mut root, &w, UiEvent::CursorMoved(at));
        assert_eq!(
            last(&ui, &root, &w),
            ("cursor\\protate".into(), 0, 400, 300)
        );
        let button = PointerButton::Left;
        // A world press is not a cursor press (only some panel
        // buttons run r5), so the type stays protate.
        send(&mut ui, &mut root, &w, UiEvent::Press { button, at });
        assert_eq!(last(&ui, &root, &w).0, "cursor\\protate");
        send(&mut ui, &mut root, &w, UiEvent::Release { button, at });
        assert_eq!(last(&ui, &root, &w).0, "cursor\\protate");
        // r8: in s = 1 past idle + 5000 ms the step starts ohand.
        w.frames += 126;
        last(&ui, &root, &w);
        assert_eq!(last(&ui, &root, &w).0, "cursor\\ohand");
    }
    // The 5 s idle is wall time: at 144 draws a second 40 ms per draw
    // showed the hand after 0.9 s.
    #[test]
    fn the_idle_hand_comes_after_five_seconds_at_25_60_and_144_draws_a_second() {
        use crate::ui::ui_clock::{draw_ms, period_ms, RATES};
        for fps in RATES {
            let (mut ui, mut root, mut w) = setup();
            ui.set_menu_wall_ms(Some(draw_ms(fps, 0)));
            send(
                &mut ui,
                &mut root,
                &w,
                UiEvent::CursorMoved(Point::new(400, 300)),
            );
            let mut i = 0;
            while last(&ui, &root, &w).0 != "cursor\\ohand" {
                i += 1;
                w.frames = i;
                ui.set_menu_wall_ms(Some(draw_ms(fps, i)));
                assert!(i < 100_000);
            }
            let ms = draw_ms(fps, i) - draw_ms(fps, 0);
            // Stepped at the first draw past 5 s, shown at the next.
            assert!(
                (5_000..=5_000 + 2 * period_ms(fps)).contains(&ms),
                "{fps}/s: {ms} ms"
            );
        }
    }

    // A tick whose draw waits for the GPU steps the cursor in its first
    // pass; the later draw of that tick still shows the pre-step frame,
    // however many passes the tick took.
    #[test]
    fn a_deferred_step_leaves_the_tick_draw_on_the_pre_step_frame() {
        let (mut ui, mut root, mut w) = setup();
        ui.set_client_seed(std::sync::Arc::new(std::sync::Mutex::new(
            crate::audio::driver::ClientSeed::default(),
        )));
        let at = Point::new(400, 300);
        send(&mut ui, &mut root, &w, UiEvent::CursorMoved(at));
        w.server_ticks = 7;
        // Draw once: the step is owed.
        let before = last(&ui, &root, &w);
        ui.hold_cursor_frame(7);
        ui.cursor_step(&w).unwrap();
        // The step ran; the same tick still draws the held frame.
        assert_eq!(last(&ui, &root, &w), before);
        // A later tick draws the stepped cursor again (no hold).
        w.server_ticks = 8;
        let _ = last(&ui, &root, &w);
    }
}
