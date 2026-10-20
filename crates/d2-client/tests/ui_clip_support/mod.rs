//! Shared body of `ui_clip_640.rs` / `ui_clip_800.rs`: the UI modules that
//! clip by `Screen::play()` (the play frame, a once-per-process choice, so
//! one binary per frame) clip to that screen.

use d2_client::bridge::world::ClientWorld;
use d2_client::ui::draw::ImageRef;
use d2_client::ui::layout::Screen;
use d2_client::ui::original::controls_host::ControlsHost;
use d2_client::ui::widget::{FrameImage, Widget};
use d2_client::ui::{NoStrings, Rect, UiCtx, UiDraw, WidgetId};

/// Every draw of the widget and the controls screen is clipped to the
/// whole screen of the play frame (the item-socket dialog that replaced
/// `imbue_ui` clips to the configured screen: `npc_talk` tests).
pub fn clips_are_the_screen(screen: Screen) {
    assert_eq!(Screen::play(), screen);
    let w = ClientWorld::default();
    let ctx = UiCtx {
        tick: 0,
        world: &w,
        strings: &NoStrings,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    let mut one: Vec<UiDraw> = Vec::new();
    FrameImage {
        id: WidgetId(1),
        rect: Rect::new(0, 0, 10, 10),
        image: ImageRef { file: 0, frame: 0 },
    }
    .draw(&mut one);
    match &one[..] {
        [UiDraw::Image(i)] => assert_eq!(i.clip, screen.rect()),
        other => panic!("{other:?}"),
    }
    ControlsHost::open(true, None).draw(&ctx, (Some(0), None), None, &mut out);
    assert!(out.len() > 3, "{} draws", out.len());
    let mut seen = [0usize; 2];
    for d in &out {
        match d {
            // The fill tiles carry their own tile clips.
            UiDraw::Image(_) => seen[0] += 1,
            UiDraw::Text(t) => {
                seen[1] += 1;
                assert_eq!(t.clip, screen.rect());
            }
            UiDraw::Rect(_) | UiDraw::Line(_) => {}
        }
    }
    assert!(seen[1] > 0, "{seen:?}");
}
