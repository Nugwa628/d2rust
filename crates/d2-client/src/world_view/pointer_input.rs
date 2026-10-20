//! Mouse button presses as the UI receives them: every press and release
//! of the left and right buttons, in the order the player made them.
//!
//! Input is read on every display frame, while the game ticks at its own
//! fixed rate (25 times a second today); the UI queue
//! ([`super::ui_bind::UiQueue`]) holds what
//! arrives between ticks until the next pass drains it, so nothing is
//! dropped while the game waits for its tick. On a fast monitor a display
//! frame is short, but a quick double-click can still put two presses
//! into one frame. Reading only the frame's button state would see one
//! press and lose the other (or report a release before the press that
//! came first), so the window's button messages are used, one event per
//! message, in order. A driver that sets the button state directly with
//! no messages (headless harnesses) still gets the state's edges.

use crate::ui::{Point, PointerButton, UiEvent};

/// One press (`down`) or release of a pointer button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonEdge {
    pub button: PointerButton,
    pub down: bool,
}

/// The frame's edges from its button messages, in order; the left and
/// right buttons only (the others are key bindings).
pub fn edges_from_messages<'a>(
    messages: impl IntoIterator<Item = &'a bevy::input::mouse::MouseButtonInput>,
) -> Vec<ButtonEdge> {
    use bevy::input::mouse::MouseButton;
    messages
        .into_iter()
        .filter_map(|m| {
            let button = match m.button {
                MouseButton::Left => PointerButton::Left,
                MouseButton::Right => PointerButton::Right,
                _ => return None,
            };
            Some(ButtonEdge {
                button,
                down: m.state.is_pressed(),
            })
        })
        .collect()
}

/// The frame's edges from the button state alone (no messages): a press,
/// then a release, per button.
pub fn edges_from_state(
    buttons: &bevy::input::ButtonInput<bevy::input::mouse::MouseButton>,
) -> Vec<ButtonEdge> {
    use bevy::input::mouse::MouseButton;
    let mut out = Vec::new();
    for (bevy, button) in [
        (MouseButton::Left, PointerButton::Left),
        (MouseButton::Right, PointerButton::Right),
    ] {
        if buttons.just_pressed(bevy) {
            out.push(ButtonEdge { button, down: true });
        }
        if buttons.just_released(bevy) {
            out.push(ButtonEdge {
                button,
                down: false,
            });
        }
    }
    out
}

/// The UI events of `edges`, in order. A press needs the pointer inside
/// the frame; a release always ends the button, at `at` (the last frame
/// position the pointer had).
pub fn pointer_events(edges: &[ButtonEdge], inside: bool, at: Point, out: &mut Vec<UiEvent>) {
    for e in edges {
        if e.down {
            if inside {
                out.push(UiEvent::Press {
                    button: e.button,
                    at,
                });
            }
        } else {
            out.push(UiEvent::Release {
                button: e.button,
                at,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::panel::UiInput;
    use bevy::ecs::entity::Entity;
    use bevy::input::mouse::{MouseButton, MouseButtonInput};
    use bevy::input::{ButtonInput, ButtonState};

    fn msg(button: MouseButton, state: ButtonState) -> MouseButtonInput {
        MouseButtonInput {
            button,
            state,
            window: Entity::PLACEHOLDER,
        }
    }

    fn click(button: MouseButton) -> [MouseButtonInput; 2] {
        [
            msg(button, ButtonState::Pressed),
            msg(button, ButtonState::Released),
        ]
    }

    /// Two clicks inside one display frame (so inside one 40 ms tick) both
    /// arrive, each press before its release, in the order made.
    #[test]
    fn two_clicks_in_one_frame_both_arrive_in_order() {
        let msgs = [click(MouseButton::Left), click(MouseButton::Right)].concat();
        let edges = edges_from_messages(&msgs);
        let at = Point::new(10, 20);
        let mut out = Vec::new();
        pointer_events(&edges, true, at, &mut out);
        assert_eq!(
            out,
            vec![
                UiEvent::Press {
                    button: PointerButton::Left,
                    at
                },
                UiEvent::Release {
                    button: PointerButton::Left,
                    at
                },
                UiEvent::Press {
                    button: PointerButton::Right,
                    at
                },
                UiEvent::Release {
                    button: PointerButton::Right,
                    at
                },
            ]
        );
    }

    /// A double-click of one button inside one frame gives two presses; the
    /// frame's button state alone would show only one.
    #[test]
    fn a_double_click_in_one_frame_gives_two_presses() {
        let msgs = [click(MouseButton::Left), click(MouseButton::Left)].concat();
        let mut out = Vec::new();
        pointer_events(
            &edges_from_messages(&msgs),
            true,
            Point::new(0, 0),
            &mut out,
        );
        let presses = out
            .iter()
            .filter(|e| matches!(e, UiEvent::Press { .. }))
            .count();
        assert_eq!(presses, 2);
        assert_eq!(out.len(), 4);
    }

    /// Clicks on display frames between two ticks queue up and reach the
    /// next pass together, in order.
    #[test]
    fn clicks_between_ticks_queue_for_the_next_pass() {
        let mut queue = super::super::ui_bind::UiQueue::default();
        for (frame, x) in [(0, 1), (1, 2), (2, 3)] {
            let edges = edges_from_messages(&click(MouseButton::Left));
            pointer_events(&edges, true, Point::new(x, frame), &mut queue.0);
        }
        let mut drained = Vec::new();
        queue.drain(&mut drained);
        let at: Vec<Point> = drained
            .iter()
            .filter_map(|e| match e {
                UiEvent::Press { at, .. } => Some(*at),
                _ => None,
            })
            .collect();
        assert_eq!(
            at,
            vec![Point::new(1, 0), Point::new(2, 1), Point::new(3, 2)]
        );
        assert_eq!(drained.len(), 6);
        assert!(queue.0.is_empty());
    }

    #[test]
    fn a_press_outside_the_frame_is_dropped_but_its_release_is_not() {
        let mut out = Vec::new();
        let at = Point::new(5, 5);
        pointer_events(
            &edges_from_messages(&click(MouseButton::Left)),
            false,
            at,
            &mut out,
        );
        assert_eq!(
            out,
            vec![UiEvent::Release {
                button: PointerButton::Left,
                at
            }]
        );
    }

    #[test]
    fn other_buttons_are_not_pointer_edges() {
        assert!(edges_from_messages(&click(MouseButton::Middle)).is_empty());
    }

    #[test]
    fn the_button_state_gives_press_then_release() {
        let mut b = ButtonInput::<MouseButton>::default();
        b.press(MouseButton::Right);
        assert_eq!(
            edges_from_state(&b),
            vec![ButtonEdge {
                button: PointerButton::Right,
                down: true
            }]
        );
        b.clear();
        b.release(MouseButton::Right);
        assert_eq!(
            edges_from_state(&b),
            vec![ButtonEdge {
                button: PointerButton::Right,
                down: false
            }]
        );
    }
}
