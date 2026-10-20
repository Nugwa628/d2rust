//! The play frame 640 × 480: the widget, imbue and controls modules clip to
//! the 640 × 480 screen, not to 800 × 600. Own binary: the play frame is a
//! once-per-process setting.

use d2_client::rules::camera::FrameSize;
use d2_client::ui::layout::Screen;

use crate::ui_clip_support;

#[test]
fn clips_follow_the_640_screen() {
    FrameSize::set_play(FrameSize::LOW).unwrap();
    ui_clip_support::clips_are_the_screen(Screen::R640);
}
