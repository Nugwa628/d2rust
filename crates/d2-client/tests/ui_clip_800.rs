//! The default play frame 800 × 600: the same modules clip to 800 × 600.
//! Own binary (see `ui_clip_640.rs`).

use d2_client::ui::layout::Screen;

use crate::ui_clip_support;

#[test]
fn clips_follow_the_800_screen() {
    ui_clip_support::clips_are_the_screen(Screen::R800);
}
