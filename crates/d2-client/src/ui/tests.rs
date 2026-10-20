//! Tests of the UI core: the panel model and routing of the root, the
//! frame mapping of window input, and the widgets.

use std::cell::RefCell;
use std::rc::Rc;

use bevy::math::{DVec2, Vec2};
use bevy::window::{Window, WindowResolution};
use d2_proto::client::Walk;
use d2_proto::PROTOCOL_VERSION;

use super::edge;
use super::widget::{Button, Cell, CellGrid, FrameImage, Label, ScrollList, TextInput, Widget};
use super::*;
use crate::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use crate::bridge::world::ClientWorld;
use crate::bridge::{Bridge, BridgeError};
use crate::ui::ui_ids::UiElement;

type Log = Rc<RefCell<Vec<(PanelId, UiEvent)>>>;

/// A panel with one button that answers every event with `answer` and
/// logs what it saw.
struct TestPanel {
    id: PanelId,
    rect: Rect,
    button: Button,
    answer: UiResponse,
    log: Log,
}

impl TestPanel {
    fn boxed(id: u16, rect: Rect, answer: UiResponse, log: &Log) -> Box<dyn Panel> {
        Box::new(Self {
            id: PanelId(id),
            rect,
            button: Button {
                id: WidgetId(id * 10),
                rect: Rect::new(rect.x, rect.y, 10, 10),
                image: Some(ImageRef {
                    file: u32::from(id),
                    frame: 0,
                }),
                pressed_image: None,
                pressed: false,
            },
            answer,
            log: log.clone(),
        })
    }
}

impl Panel for TestPanel {
    fn id(&self) -> PanelId {
        self.id
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn draw(&self, _ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        self.button.draw(out);
    }
    fn hit(&self, p: Point) -> Option<WidgetId> {
        self.button.hit(p)
    }
    fn ui_elements(&self, _ctx: &UiCtx) -> Vec<UiElement> {
        vec![UiElement::new(
            format!("test.button({})", self.id.0),
            self.button.rect,
        )]
    }
    fn event(&mut self, e: UiEvent, _ctx: &UiCtx) -> UiResponse {
        self.log.borrow_mut().push((self.id, e));
        self.answer.clone()
    }
}

fn with_ctx<R>(f: impl FnOnce(&UiCtx) -> R) -> R {
    let world = ClientWorld::default();
    let ctx = UiCtx {
        tick: 0,
        world: &world,
        strings: &NoStrings,
    };
    f(&ctx)
}

fn press(x: i32, y: i32) -> UiEvent {
    UiEvent::Press {
        button: PointerButton::Left,
        at: Point::new(x, y),
    }
}

/// Panel 1 at (0,0) 200×200 below panel 2 at (100,100) 200×200, both open.
fn overlapping(a1: UiResponse, a2: UiResponse) -> (UiRoot, Log) {
    let log = Log::default();
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    root.add(TestPanel::boxed(1, Rect::new(0, 0, 200, 200), a1, &log))
        .unwrap();
    root.add(TestPanel::boxed(2, Rect::new(100, 100, 200, 200), a2, &log))
        .unwrap();
    root.open(PanelId(1)).unwrap();
    root.open(PanelId(2)).unwrap();
    (root, log)
}

// ---- frame mapping ----

#[test]
fn scale_two_corner_maps_to_last_pixel() {
    let p = Presentation::new(1600, 1200).unwrap();
    assert_eq!((p.scale, p.left, p.top), (2, 0, 0));
    assert_eq!(
        p.to_frame(1599, 1199),
        FramePos::Inside(Point::new(799, 599))
    );
    assert_eq!(p.to_frame(0, 0), FramePos::Inside(Point::new(0, 0)));
    assert_eq!(p.to_frame(1, 1), FramePos::Inside(Point::new(0, 0)));
    assert_eq!(p.to_frame(2, 3), FramePos::Inside(Point::new(1, 1)));
}

#[test]
fn black_bars_are_outside() {
    let p = Presentation::new(1700, 1200).unwrap();
    assert_eq!((p.scale, p.left, p.top), (2, 50, 0));
    assert_eq!(p.to_frame(25, 0), FramePos::Outside);
    assert_eq!(p.to_frame(49, 0), FramePos::Outside);
    assert_eq!(p.to_frame(50, 0), FramePos::Inside(Point::new(0, 0)));
    assert_eq!(
        p.to_frame(1649, 1199),
        FramePos::Inside(Point::new(799, 599))
    );
    assert_eq!(p.to_frame(1650, 0), FramePos::Outside);
    // Outside the window entirely.
    assert_eq!(p.to_frame(-1, 5), FramePos::Outside);
    assert_eq!(p.to_frame(100, 1200), FramePos::Outside);
}

#[test]
fn presentation_scale_and_bars() {
    let p = Presentation::new(800, 600).unwrap();
    assert_eq!((p.scale, p.left, p.top), (1, 0, 0));
    assert_eq!(p.to_frame(799, 599), FramePos::Inside(Point::new(799, 599)));
    assert_eq!(p.to_frame(800, 0), FramePos::Outside);
    // Scale limited by the height; odd remainder rounds the left bar down.
    let p = Presentation::new(2401, 1300).unwrap();
    assert_eq!((p.scale, p.left, p.top), (2, 400, 50));
    assert_eq!(p.to_frame(400, 50), FramePos::Inside(Point::new(0, 0)));
    assert_eq!(p.to_frame(399, 50), FramePos::Outside);
    assert_eq!(
        Presentation::new(799, 600),
        Err(FrameError::TooSmall { w: 799, h: 600 })
    );
    assert_eq!(
        Presentation::new(1600, 599),
        Err(FrameError::TooSmall { w: 1600, h: 599 })
    );
}

#[test]
fn bevy_edge_reads_physical_cursor() {
    let mut w = Window {
        resolution: WindowResolution::new(1600, 1200).with_scale_factor_override(1.0),
        ..Default::default()
    };
    assert_eq!(edge::cursor_frame_pos(&w), Ok(FramePos::Outside));
    w.set_physical_cursor_position(Some(DVec2::new(1599.75, 1199.5)));
    assert_eq!(
        edge::cursor_frame_pos(&w),
        Ok(FramePos::Inside(Point::new(799, 599)))
    );
    assert_eq!(edge::window_pixel(Vec2::new(-0.5, 2.9)), (-1, 2));
    let small = Window {
        resolution: WindowResolution::new(640, 480).with_scale_factor_override(1.0),
        ..Default::default()
    };
    // A window smaller than the frame shows it scaled down (fit), so the
    // cursor still maps; an empty window has no presentation.
    assert_eq!(edge::cursor_frame_pos(&small), Ok(FramePos::Outside));
    let empty = Window {
        resolution: WindowResolution::new(0, 0).with_scale_factor_override(1.0),
        ..Default::default()
    };
    assert!(edge::cursor_frame_pos(&empty).is_err());
}

// ---- routing ----

#[test]
fn overlapping_panels_top_most_gets_the_event() {
    let (mut root, log) = overlapping(UiResponse::Consumed, UiResponse::Consumed);
    with_ctx(|ctx| {
        assert_eq!(
            root.dispatch(press(150, 150), ctx),
            Routed::Panel(PanelId(2))
        );
        assert_eq!(
            root.hit(Point::new(150, 150)),
            Some(UiHit {
                panel: PanelId(2),
                widget: None
            })
        );
        // Only the lower panel covers (50,50).
        assert_eq!(root.dispatch(press(50, 50), ctx), Routed::Panel(PanelId(1)));
        assert_eq!(root.dispatch(press(500, 500), ctx), Routed::Unhandled);
    });
    assert_eq!(
        *log.borrow(),
        vec![(PanelId(2), press(150, 150)), (PanelId(1), press(50, 50))]
    );
}

#[test]
fn ignored_event_walks_down_then_is_unhandled() {
    let (mut root, log) = overlapping(UiResponse::Consumed, UiResponse::Ignored);
    with_ctx(|ctx| {
        assert_eq!(
            root.dispatch(press(150, 150), ctx),
            Routed::Panel(PanelId(1))
        );
        // Panel 2 alone covers (250,250) and ignores it.
        assert_eq!(root.dispatch(press(250, 250), ctx), Routed::Unhandled);
    });
    assert_eq!(
        *log.borrow(),
        vec![
            (PanelId(2), press(150, 150)),
            (PanelId(1), press(150, 150)),
            (PanelId(2), press(250, 250)),
        ]
    );
}

#[test]
fn closed_panels_get_nothing_and_draw_nothing() {
    let (mut root, log) = overlapping(UiResponse::Consumed, UiResponse::Consumed);
    root.close(PanelId(2)).unwrap();
    with_ctx(|ctx| {
        assert_eq!(
            root.dispatch(press(150, 150), ctx),
            Routed::Panel(PanelId(1))
        );
        assert_eq!(root.dispatch(press(250, 250), ctx), Routed::Unhandled);
        let mut out = Vec::new();
        root.draw(ctx, &mut out);
        assert_eq!(out.len(), 1);
    });
    assert_eq!(log.borrow().len(), 1);
}

#[test]
fn non_pointer_events_go_top_most_first_to_all_open_panels() {
    let (mut root, log) = overlapping(UiResponse::Consumed, UiResponse::Ignored);
    with_ctx(|ctx| {
        let a = UiEvent::Action(ActionId(3));
        assert_eq!(root.dispatch(a, ctx), Routed::Panel(PanelId(1)));
        assert_eq!(*log.borrow(), vec![(PanelId(2), a), (PanelId(1), a)]);
    });
    let (mut root, _) = overlapping(UiResponse::Ignored, UiResponse::Ignored);
    with_ctx(|ctx| assert_eq!(root.dispatch(UiEvent::Char(0x41), ctx), Routed::Unhandled));
}

#[test]
fn draw_is_bottom_most_first() {
    let (root, _) = overlapping(UiResponse::Consumed, UiResponse::Consumed);
    let mut out = Vec::new();
    with_ctx(|ctx| root.draw(ctx, &mut out));
    let files: Vec<u32> = out
        .iter()
        .map(|d| match d {
            UiDraw::Image(i) => i.image.file,
            UiDraw::Text(_) => panic!("text"),
            UiDraw::Rect(_) => panic!("rectangle"),
            UiDraw::Line(_) => panic!("line"),
        })
        .collect();
    assert_eq!(files, vec![1, 2]);
}

#[test]
fn hover_follows_cursor_and_widgets() {
    let (mut root, _) = overlapping(UiResponse::Ignored, UiResponse::Ignored);
    with_ctx(|ctx| {
        assert_eq!(root.hovered(), None);
        root.dispatch(UiEvent::CursorMoved(Point::new(105, 105)), ctx);
        assert_eq!(
            root.hovered(),
            Some(UiHit {
                panel: PanelId(2),
                widget: Some(WidgetId(20))
            })
        );
        root.close(PanelId(2)).unwrap();
        assert_eq!(
            root.hovered(),
            Some(UiHit {
                panel: PanelId(1),
                widget: None
            })
        );
        root.dispatch(UiEvent::CursorLeft, ctx);
        assert_eq!(root.hovered(), None);
    });
}

#[test]
fn panel_ids_are_strict() {
    let log = Log::default();
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    root.add(TestPanel::boxed(1, FRAME, UiResponse::Ignored, &log))
        .unwrap();
    assert_eq!(
        root.add(TestPanel::boxed(1, FRAME, UiResponse::Ignored, &log)),
        Err(UiError::DuplicatePanel(PanelId(1)))
    );
    assert_eq!(
        root.open(PanelId(9)),
        Err(UiError::UnknownPanel(PanelId(9)))
    );
    assert_eq!(
        root.close(PanelId(9)),
        Err(UiError::UnknownPanel(PanelId(9)))
    );
    assert_eq!(
        root.is_open(PanelId(9)),
        Err(UiError::UnknownPanel(PanelId(9)))
    );
    assert_eq!(root.is_open(PanelId(1)), Ok(false));
    root.toggle(PanelId(1)).unwrap();
    assert_eq!(root.open_panels(), vec![PanelId(1)]);
    root.toggle(PanelId(1)).unwrap();
    assert!(root.open_panels().is_empty());
}

/// Closes every listed panel when `opening` opens.
struct Excludes(Vec<(PanelId, Vec<PanelId>)>, Rc<RefCell<Vec<Vec<PanelId>>>>);

impl PanelRules for Excludes {
    fn on_open(&mut self, opening: PanelId, open: &[PanelId]) -> Vec<PanelId> {
        self.1.borrow_mut().push(open.to_vec());
        self.0
            .iter()
            .find(|(p, _)| *p == opening)
            .map(|(_, c)| c.clone())
            .unwrap_or_default()
    }
}

#[test]
fn panel_rules_hook_closes_and_is_strict() {
    let log = Log::default();
    let seen = Rc::default();
    let rules = Excludes(
        vec![
            (PanelId(3), vec![PanelId(1)]),
            (PanelId(4), vec![PanelId(1), PanelId(4)]),
            (PanelId(5), vec![PanelId(1), PanelId(77)]),
        ],
        Rc::clone(&seen),
    );
    let mut root = UiRoot::new(Box::new(rules));
    for id in 1..=5 {
        root.add(TestPanel::boxed(id, FRAME, UiResponse::Ignored, &log))
            .unwrap();
    }
    root.open(PanelId(1)).unwrap();
    root.open(PanelId(2)).unwrap();
    root.open(PanelId(3)).unwrap();
    assert_eq!(root.open_panels(), vec![PanelId(2), PanelId(3)]);
    // Already open: rules not consulted.
    root.open(PanelId(3)).unwrap();
    assert_eq!(
        *seen.borrow(),
        vec![vec![], vec![PanelId(1)], vec![PanelId(1), PanelId(2)]]
    );
    root.open(PanelId(1)).unwrap();
    // A rule that closes the opening panel, or names an unknown one, is an
    // error and changes nothing.
    assert_eq!(
        root.open(PanelId(4)),
        Err(UiError::RuleClosedOpening { closed: PanelId(4) })
    );
    assert_eq!(
        root.open(PanelId(5)),
        Err(UiError::UnknownPanel(PanelId(77)))
    );
    assert_eq!(root.open_panels(), vec![PanelId(1), PanelId(2), PanelId(3)]);
}

// ---- intents ----

struct RecordLink(Rc<RefCell<Vec<Vec<u8>>>>);

impl ServerLink for RecordLink {
    fn protocol_version(&self) -> u32 {
        PROTOCOL_VERSION
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn send(&mut self, _queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.0.borrow_mut().push(msg.to_vec());
        Ok(Sent::Queued)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        Ok(Pumped { ticked: false })
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        Vec::new()
    }
}

#[test]
fn only_the_root_forwards_intents_in_order() {
    let walk = ClientIntent::from_message(&Walk { x: 7, y: 9 });
    let (mut root, _) = overlapping(UiResponse::Consumed, UiResponse::Intent(walk.clone()));
    with_ctx(|ctx| {
        assert_eq!(
            root.dispatch(press(150, 150), ctx),
            Routed::Panel(PanelId(2))
        );
        assert_eq!(
            root.dispatch(press(250, 250), ctx),
            Routed::Panel(PanelId(2))
        );
    });
    assert_eq!(root.intents(), &[walk.clone(), walk.clone()]);

    let sent = Rc::default();
    let mut bridge = Bridge::new(RecordLink(Rc::clone(&sent))).unwrap();
    assert_eq!(root.forward(&mut bridge).unwrap(), 2);
    assert!(root.intents().is_empty());
    assert_eq!(*sent.borrow(), vec![walk.0.clone(), walk.0.clone()]);
}

/// A frame-sized panel answering events with a fixed list of intents.
struct ListPanel {
    id: PanelId,
    answers: std::collections::VecDeque<ClientIntent>,
}

#[test]
fn refused_intent_stays_queued() {
    let walk = ClientIntent::from_message(&Walk { x: 1, y: 2 });
    let bad = ClientIntent(vec![0x80; 5]);
    let mut r = UiRoot::new(Box::new(NoPanelRules));
    r.add(Box::new(ListPanel {
        id: PanelId(1),
        answers: [walk.clone(), bad.clone(), walk.clone()].into(),
    }))
    .unwrap();
    r.open(PanelId(1)).unwrap();
    with_ctx(|ctx| {
        for _ in 0..3 {
            assert_eq!(r.dispatch(press(1, 1), ctx), Routed::Panel(PanelId(1)));
        }
    });
    let sent = Rc::default();
    let mut bridge = Bridge::new(RecordLink(Rc::clone(&sent))).unwrap();
    assert!(matches!(
        r.forward(&mut bridge),
        Err(BridgeError::Intent(_))
    ));
    assert_eq!(*sent.borrow(), vec![walk.0.clone()]);
    assert_eq!(r.intents(), &[bad, walk]);
}

impl Panel for ListPanel {
    fn id(&self) -> PanelId {
        self.id
    }
    fn rect(&self) -> Rect {
        FRAME
    }
    fn draw(&self, _ctx: &UiCtx, _out: &mut dyn UiDrawSink) {}
    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }
    fn event(&mut self, _e: UiEvent, _ctx: &UiCtx) -> UiResponse {
        self.answers
            .pop_front()
            .map_or(UiResponse::Ignored, UiResponse::Intent)
    }
}

struct Script(Vec<UiEvent>);

impl UiInput for Script {
    fn drain(&mut self, out: &mut Vec<UiEvent>) {
        out.append(&mut self.0);
    }
}

#[test]
fn pump_returns_unhandled_events_in_order() {
    let (mut root, _) = overlapping(UiResponse::Consumed, UiResponse::Consumed);
    let events = vec![
        UiEvent::CursorMoved(Point::new(700, 10)),
        press(150, 150),
        press(700, 10),
        UiEvent::Wheel {
            steps: -1,
            at: Point::new(10, 10),
        },
    ];
    let mut input = Script(events.clone());
    let left = with_ctx(|ctx| root.pump(&mut input, ctx));
    assert_eq!(left, vec![events[0], events[2]]);
    assert!(input.0.is_empty());
}

// ---- widgets ----

#[test]
fn rect_hit_is_half_open() {
    let r = Rect::new(10, 20, 30, 40);
    assert!(r.contains(Point::new(10, 20)));
    assert!(r.contains(Point::new(39, 59)));
    assert!(!r.contains(Point::new(40, 20)));
    assert!(!r.contains(Point::new(10, 60)));
    assert!(!r.contains(Point::new(9, 20)));
    assert!(!Rect::new(0, 0, 0, 5).contains(Point::new(0, 0)));
}

#[test]
fn widgets_emit_requests_and_hit_by_rect() {
    let img = ImageRef { file: 4, frame: 2 };
    let style = TextStyle { font: 1, color: 3 };
    let b = Button {
        id: WidgetId(1),
        rect: Rect::new(5, 6, 7, 8),
        image: None,
        pressed_image: None,
        pressed: false,
    };
    let f = FrameImage {
        id: WidgetId(2),
        rect: Rect::new(1, 1, 2, 2),
        image: img,
    };
    let l = Label {
        id: WidgetId(3),
        rect: Rect::new(0, 50, 100, 10),
        pen: Point::new(4, 59),
        text: vec![0x48, 0x69],
        style,
    };
    let mut out = Vec::new();
    b.draw(&mut out);
    f.draw(&mut out);
    l.draw(&mut out);
    assert_eq!(
        out,
        vec![
            UiDraw::Image(ImageRequest {
                image: img,
                at: Point::new(1, 1),
                clip: FRAME,
                look: crate::ui::CelLook::PLAIN,
                call: crate::ui::draw::CelCall::Draw,
            }),
            UiDraw::Text(TextRequest {
                text: vec![0x48, 0x69],
                at: Point::new(4, 59),
                style,
                opts: TextOpts::default(),
                clip: FRAME
            }),
        ]
    );
    assert_eq!(b.hit(Point::new(11, 13)), Some(WidgetId(1)));
    assert_eq!(b.hit(Point::new(12, 13)), None);
    assert_eq!(l.hit(Point::new(99, 59)), Some(WidgetId(3)));
}

#[test]
fn cell_grid_integer_hit() {
    let g = CellGrid::new(WidgetId(1), Point::new(100, 200), 10, 4, 29, 29).unwrap();
    assert_eq!(g.rect(), Rect::new(100, 200, 290, 116));
    assert_eq!(
        g.cell_at(Point::new(100, 200)),
        Some(Cell { col: 0, row: 0 })
    );
    assert_eq!(
        g.cell_at(Point::new(128, 228)),
        Some(Cell { col: 0, row: 0 })
    );
    assert_eq!(
        g.cell_at(Point::new(129, 229)),
        Some(Cell { col: 1, row: 1 })
    );
    assert_eq!(
        g.cell_at(Point::new(389, 315)),
        Some(Cell { col: 9, row: 3 })
    );
    assert_eq!(g.cell_at(Point::new(390, 315)), None);
    assert_eq!(g.cell_at(Point::new(99, 200)), None);
    assert_eq!(
        g.cell_rect(Cell { col: 2, row: 1 }),
        Some(Rect::new(158, 229, 29, 29))
    );
    assert_eq!(g.cell_rect(Cell { col: 10, row: 0 }), None);
    assert!(CellGrid::new(WidgetId(2), Point::new(0, 0), 0, 4, 29, 29).is_err());
    assert!(CellGrid::new(WidgetId(2), Point::new(0, 0), 4, 4, 29, 0).is_err());
    assert!(CellGrid::new(WidgetId(2), Point::new(0, 0), 1000, 1, 100, 1).is_err());
}

#[test]
fn scroll_list_rows_and_clamp() {
    let mut s = ScrollList::new(WidgetId(1), Rect::new(0, 0, 100, 50), 10).unwrap();
    assert_eq!(s.rows_visible(), 5);
    assert_eq!(s.row_at(Point::new(5, 5)), None);
    s.set_len(8);
    assert_eq!(s.row_at(Point::new(5, 49)), Some(4));
    s.scroll(10);
    assert_eq!(s.first(), 3);
    assert_eq!(s.row_at(Point::new(5, 0)), Some(3));
    assert_eq!(s.row_at(Point::new(5, 49)), Some(7));
    s.scroll(-1);
    assert_eq!(s.first(), 2);
    s.scroll(-100);
    assert_eq!(s.first(), 0);
    s.scroll(3);
    s.set_len(6);
    assert_eq!(s.first(), 1);
    s.set_len(2);
    assert_eq!(s.first(), 0);
    assert_eq!(s.row_at(Point::new(5, 25)), None);
    // A 55-high list shows 5 whole rows; the last 5 pixels are no row.
    let s = ScrollList::new(WidgetId(1), Rect::new(0, 0, 10, 55), 10).unwrap();
    assert_eq!(s.rows_visible(), 5);
    assert!(ScrollList::new(WidgetId(1), Rect::new(0, 0, 10, 5), 10).is_err());
    assert!(ScrollList::new(WidgetId(1), Rect::new(0, 0, 10, 5), 0).is_err());
}

#[test]
fn text_input_keeps_code_units() {
    let mut t = TextInput::new(
        WidgetId(1),
        Rect::new(0, 0, 50, 10),
        TextStyle::default(),
        2,
    );
    assert!(!t.backspace());
    assert!(t.insert(0x00FF));
    assert!(t.insert(0x0063));
    assert!(!t.insert(0x0031));
    assert_eq!(t.text(), &[0x00FF, 0x0063]);
    assert!(t.backspace());
    assert_eq!(t.take(), vec![0x00FF]);
    assert!(t.text().is_empty());
}

#[test]
fn one_logical_frame_of_800_by_600() {
    assert_eq!((FRAME_W, FRAME_H), (800, 600));
    assert_eq!(FRAME, Rect::new(0, 0, 800, 600));
    // Every window shows the same 800×600 frame, only scaled: its last
    // image pixel is frame pixel (799, 599) whatever the window size.
    for (w, h) in [
        (800, 600),
        (1024, 768),
        (1280, 720),
        (1920, 1080),
        (2560, 1440),
    ] {
        let p = Presentation::new(w, h).unwrap();
        let (x1, y1) = (p.left + 800 * p.scale - 1, p.top + 600 * p.scale - 1);
        assert_eq!(
            p.to_frame(i64::from(x1), i64::from(y1)),
            FramePos::Inside(Point::new(799, 599)),
            "{w}×{h}"
        );
    }
    // No 640×480 layout: a window that cannot hold 800×600 is refused.
    assert_eq!(
        Presentation::new(640, 480),
        Err(FrameError::TooSmall { w: 640, h: 480 })
    );
}

// --- Text --------------------------------------------------------------

mod text {
    use d2_formats::font::{FontTable, Glyph};

    use super::super::text::*;
    use super::*;

    const TSV: &str = include_str!("../../../../tables/ui/text-fonts.tsv");

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn s(units: &[u16]) -> String {
        String::from_utf16(units).unwrap()
    }

    /// A 256-record Latin font (record `i`: `frame = i`), every advance
    /// `default` except `advs`. The record `code` fields are scrambled:
    /// the lookup is by position and never reads them.
    fn font(height: u8, default: u8, advs: &[(char, u8)]) -> FontTable {
        let glyphs = (0..256u16)
            .map(|i| Glyph {
                code: 255 - i,
                unknown1: 0,
                width: advs
                    .iter()
                    .find(|(c, _)| *c as u16 == i)
                    .map_or(default, |&(_, w)| w),
                height,
                unknown2: 1,
                unknown3: 0,
                frame: i,
                unknown5: 0,
            })
            .collect();
        FontTable {
            version: 1,
            unknown: 0,
            count: 256,
            height,
            width: 0,
            glyphs,
        }
    }

    /// Font16 advances the test vectors use (`A` 12, `B` 7, `a`, `ÿ`, `x`
    /// 10; `Stash` = 7, 9, 10, 7, 7), height 10.
    fn font16() -> FontTable {
        let advs = [
            ('A', 12),
            ('B', 7),
            ('C', 9),
            ('a', 10),
            ('ÿ', 10),
            ('x', 10),
            ('b', 10),
            ('S', 7),
            ('t', 9),
            ('s', 7),
            ('h', 7),
        ];
        font(10, 6, &advs)
    }

    /// Font8 advances meeting the width vectors (`Gold` 28, `Rare` 29,
    /// `ÿc4` = `ÿc9` = 21, `m` 12), height 14.
    fn font8() -> FontTable {
        let advs = [
            ('G', 8),
            ('o', 7),
            ('l', 5),
            ('d', 8),
            ('R', 8),
            ('a', 7),
            ('r', 6),
            ('e', 8),
            ('ÿ', 7),
            ('c', 7),
            ('4', 7),
            ('9', 7),
            ('m', 12),
            ('X', 9),
        ];
        font(14, 6, &advs)
    }

    /// Every advance 5, height 10.
    fn five() -> FontTable {
        font(10, 5, &[])
    }

    fn place(
        f: &FontTable,
        text: &str,
        at: Point,
        color: u16,
        opts: TextOpts,
    ) -> Vec<(char, i32, i32, i32)> {
        let style = TextStyle { font: 1, color };
        OriginalText
            .place(&GlyphLookup::new(f), &u(text), at, style, &opts)
            .unwrap()
            .into_iter()
            .map(|p| {
                (
                    char::from_u32(u32::from(p.code)).unwrap(),
                    p.at.x,
                    p.at.y,
                    p.color,
                )
            })
            .collect()
    }

    #[test]
    fn font_table_matches_text_fonts_tsv() {
        let mut rows = TSV.lines();
        assert_eq!(
            rows.next().unwrap(),
            "id\tname\ttbl_path\tdc6_path\tdc6_archive\theight\tline_step\tframe_w\tframe_h"
        );
        let mut n = 0;
        for (row, f) in rows.zip(FONTS.iter()) {
            let c: Vec<&str> = row.split('\t').collect();
            let got = [
                f.id.to_string(),
                f.name.into(),
                f.tbl_path.into(),
                f.dc6_path.into(),
                f.dc6_archive.into(),
                f.height.to_string(),
                f.line_step.to_string(),
                f.frame_w.to_string(),
                f.frame_h.to_string(),
            ];
            assert_eq!(c, got, "row {n}");
            // line_step = trunc(height × 16 / 10).
            assert_eq!(f.line_step, i32::from(f.height) * 16 / 10, "row {n}");
            assert_eq!(font_info(f.id), Some(f));
            n += 1;
        }
        assert_eq!((n, TSV.lines().count()), (14, 15));
        assert_eq!(font_info(14), None);
    }

    #[test]
    fn latin_lookup_is_by_position() {
        let f = font16();
        let g = GlyphLookup::new(&f);
        assert_eq!(g.record(0x20AC), Ok(0));
        assert_eq!(g.record(0x0041), Ok(65));
        assert_eq!(g.record(0x00FF), Ok(255));
        // The record's `code` field (here 255 − i) is not read.
        assert_eq!(f.glyphs[65].code, 190);
        assert_eq!(g.adv(u16::from(b'A')), Ok(12));
        // A font with fewer records than the position: malformed input.
        let mut short = five();
        short.glyphs.truncate(65);
        assert_eq!(
            GlyphLookup::new(&short).record(0x41),
            Err(TextError::MissingGlyph {
                code: 0x41,
                record: 65
            })
        );
        assert_eq!(GlyphLookup::new(&short).record(0x20AC), Ok(0));
    }

    #[test]
    fn measures_follow_their_own_code_rules() {
        let f16 = font16();
        let g = GlyphLookup::new(&f16);
        assert_eq!(width_a(&g, &u("Stash")), Ok(40));

        let f8 = font8();
        let g = GlyphLookup::new(&f8);
        let gold = u("ÿc4Gold");
        assert_eq!(width_a(&g, &gold), Ok(49));
        assert_eq!(width_c(&g, &gold, 0, 7), Ok(28));
        assert_eq!(line_width(&g, &gold, 0), Ok(28));
        assert_eq!(max_width(&g, &gold), Ok(28));
        // Width C skips only codes 0–6...
        let rare = u("ÿc9Rare");
        assert_eq!(width_c(&g, &rare, 0, 7), Ok(50));
        assert_eq!(line_width(&g, &rare, 0), Ok(29));
        // ... and only when the code starts at i with i + 3 < n.
        assert_eq!(width_c(&g, &gold, 0, 3), Ok(21));
        assert_eq!(width_c(&g, &gold, 0, 4), Ok(8));
        // `ÿm` / `ÿM`: zero for the three units plus adv('m').
        assert_eq!(line_width(&g, &u("ÿmX"), 0), Ok(12));
        assert_eq!(line_width(&g, &u("ÿMX"), 0), Ok(12));
        // Width B: the first n units, codes counted; LF is 0.
        assert_eq!(width_b(&g, &gold, 3), Ok(21));
        assert_eq!(width_b(&g, &u("G\nG"), 3), Ok(16));
        // Max width: the widest LF line; line width from a start.
        let two = u("Gold\nRare");
        assert_eq!(max_width(&g, &two), Ok(29));
        assert_eq!(line_width(&g, &two, 5), Ok(29));
        // A NUL ends the string.
        assert_eq!(width_a(&g, &[u16::from(b'G'), 0, u16::from(b'G')]), Ok(8));
    }

    #[test]
    fn measures_lf_and_skip_rules() {
        let f = five();
        let g = GlyphLookup::new(&f);
        // `LF` adds 0 in widths A, B and C (not record 10).
        let t = u("a\nb");
        assert_eq!(width_a(&g, &t), Ok(10));
        assert_eq!(width_b(&g, &t, 3), Ok(10));
        assert_eq!(width_c(&g, &t, 0, 3), Ok(10));
        // Width C stops at the NUL: `n` past the end adds nothing.
        assert_eq!(width_c(&g, &t, 0, 9), Ok(10));
        // Line width: `ÿ` and the next two units are passed over unread, so
        // the `LF` among them ends nothing: 5 + 0 + 5; max width the same.
        let skip = u("xÿ\nyz");
        assert_eq!(line_width(&g, &skip, 0), Ok(10));
        assert_eq!(max_width(&g, &skip), Ok(10));
        // A skip past the end ends the walk.
        assert_eq!(line_width(&g, &u("abÿ"), 0), Ok(10));
        assert_eq!(max_width(&g, &u("abÿ\n")), Ok(10));
    }

    #[test]
    fn text_height_and_line_step() {
        let f = font16();
        let g = GlyphLookup::new(&f);
        assert_eq!(text_height(&g, &u("AB\nC")), 32);
        assert_eq!(text_height(&g, &u("AB")), 16);
        assert_eq!(g.line_step(), 16);
        let f8 = font8();
        // trunc(14 × 16 / 10) = 22, trunc(14 × 16 × 3 / 10) = 67.
        let g8 = GlyphLookup::new(&f8);
        assert_eq!((g8.line_step(), text_height(&g8, &u("a\nb\nc"))), (22, 67));
    }

    #[test]
    fn draw_centered_lines_go_up() {
        let f = font16();
        let g = GlyphLookup::new(&f);
        assert_eq!(max_width(&g, &u("AB\nC")), Ok(19));
        assert_eq!(
            place(&f, "AB\nC", Point::new(100, 200), 0, TextOpts::centered()),
            [('A', 104, 200, 0), ('B', 116, 200, 0), ('C', 109, 184, 0)]
        );
        // A one-line string is drawn at x + 4.
        assert_eq!(
            place(&f, "A", Point::new(0, 0), 0, TextOpts::centered()),
            [('A', 4, 0, 0)]
        );
        // An explicit block width replaces max width + 8.
        let opts = TextOpts::Draw {
            centered: true,
            block_w: Some(20),
            mode: 5,
        };
        assert_eq!(place(&f, "A", Point::new(0, 0), 0, opts), [('A', 4, 0, 0)]);
    }

    #[test]
    fn color_codes_switch_color_across_line_feeds() {
        let f = font16();
        assert_eq!(
            place(&f, "Aÿc1B\nC", Point::new(10, 50), 0, TextOpts::default()),
            [('A', 10, 50, 0), ('B', 22, 50, 1), ('C', 10, 34, 1)]
        );
        // Each call starts with the caller's color; `ÿcB` is a code
        // (k = 18 ≥ 13 → 0) taking three units, so nothing follows `A`.
        assert_eq!(
            place(&f, "AÿcB", Point::new(0, 0), 7, TextOpts::default()),
            [('A', 0, 0, 7)]
        );
        // Synthetic advance 5, height 10 (line step 16).
        assert_eq!(
            place(
                &five(),
                "ab\ncd",
                Point::new(0, 100),
                0,
                TextOpts::default()
            ),
            [
                ('a', 0, 100, 0),
                ('b', 5, 100, 0),
                ('c', 0, 84, 0),
                ('d', 5, 84, 0)
            ]
        );
    }

    #[test]
    fn color_code_values_and_ends() {
        let f = five();
        let k = |t: &str| place(&f, t, Point::new(0, 0), 9, TextOpts::default())[0].3;
        assert_eq!(k("ÿc=a"), 0);
        assert_eq!(k("ÿc<a"), 12);
        assert_eq!(k("ÿc0a"), 0);
        assert_eq!(k("ÿc;a"), 11);
        // Below `0`: a negative k is kept.
        assert_eq!(k("ÿc/a"), -1);
        // `ÿ` + anything else is the glyph of record 255.
        assert_eq!(
            place(&font16(), "aÿxb", Point::new(0, 20), 0, TextOpts::default()),
            [
                ('a', 0, 20, 0),
                ('ÿ', 10, 20, 0),
                ('x', 20, 20, 0),
                ('b', 30, 20, 0)
            ]
        );
        // Case-sensitive: `ÿC` is a glyph.
        assert_eq!(
            place(&f, "ÿC1", Point::new(0, 0), 0, TextOpts::default()).len(),
            3
        );
        // `ÿ` or `ÿc` at the end: nothing drawn, drawing ends.
        assert_eq!(
            place(&f, "aÿ", Point::new(0, 0), 0, TextOpts::default()).len(),
            1
        );
        assert_eq!(
            place(&f, "aÿc", Point::new(0, 0), 0, TextOpts::default()).len(),
            1
        );
    }

    #[test]
    fn centered_span_counts_codes() {
        let f = font16();
        let g = GlyphLookup::new(&f);
        // span 100, width A 40: x = 1 + 30.
        assert_eq!(centered_span_x(&g, &u("Stash"), 1, 100), Ok(31));
        // Wider than the span: x1.
        assert_eq!(centered_span_x(&g, &u("Stash"), 1, 30), Ok(1));
        let f8 = font8();
        let g8 = GlyphLookup::new(&f8);
        // `ÿc4Gold`: width A 49 (codes counted), not the visible 28.
        assert_eq!(centered_span_x(&g8, &u("ÿc4Gold"), 0, 99), Ok(25));
    }

    #[test]
    fn framed_hover_box() {
        let f = font16();
        let g = GlyphLookup::new(&f);
        let text = u("AB\nC");
        let fr = framed_text(&g, &text, Point::new(790, 100), (800, 600)).unwrap();
        assert_eq!(fr.rect, (Point::new(773, 70), Point::new(800, 102)));
        assert_eq!(fr.pen, Point::new(773, 99));
        assert_eq!(
            place(&f, "AB\nC", fr.pen, 0, fr.opts),
            [('A', 777, 99, 0), ('B', 789, 99, 0), ('C', 782, 83, 0)]
        );
        // Left of the screen: x' = 0; low on the screen: bottom Sh − 31.
        let fr = framed_text(&g, &text, Point::new(-5, 590), (800, 600)).unwrap();
        assert_eq!(fr.rect, (Point::new(0, 537), Point::new(27, 569)));
        // Near the top: bottom = text height.
        let fr = framed_text(&g, &text, Point::new(0, 0), (800, 600)).unwrap();
        assert_eq!(fr.rect, (Point::new(0, 0), Point::new(27, 32)));
    }

    #[test]
    fn framed_tight_variant() {
        let f = font16();
        let g = GlyphLookup::new(&f);
        let text = u("AB\nC");
        let fr = framed_text_tight(&g, &text, Point::new(790, 100), (800, 600)).unwrap();
        assert_eq!(fr.rect, (Point::new(781, 92), Point::new(800, 102)));
        assert_eq!(fr.pen, Point::new(781, 100));
        assert_eq!(
            place(&f, "AB\nC", fr.pen, 0, fr.opts)[0],
            ('A', 781, 100, 0)
        );
        // Box above row 0: y' doubles (an original bug).
        let fr = framed_text_tight(&g, &text, Point::new(0, -5), (800, 600)).unwrap();
        assert_eq!(fr.rect, (Point::new(0, -16), Point::new(19, -6)));
        let fr = framed_text_tight(&g, &text, Point::new(0, 700), (800, 600)).unwrap();
        assert_eq!(fr.rect.1, Point::new(19, 569));
    }

    // (the stamina tip at (W/2 − 76, H − 52): box centred on W/2 − 76,
    // bottom H − 50)
    #[test]
    fn popup_text_is_centred_on_x_with_its_bottom_at_y_plus_2() {
        let f = font16();
        let g = GlyphLookup::new(&f);
        let text = u("AB");
        // W = 19 + 8 = 27; x0 = 324 − 13; y0 = b = 550; text at b − 3.
        let fr = popup_text(&g, &text, Point::new(324, 548), true, (800, 600)).unwrap();
        assert_eq!(fr.rect, (Point::new(311, 534), Point::new(338, 550)));
        assert_eq!(fr.pen, Point::new(311, 547));
        assert_eq!(
            place(&f, "AB", fr.pen, 0, fr.opts),
            [('A', 315, 547, 0), ('B', 327, 547, 0)]
        );
        // centre 0: x0 = x; x0 ≤ 0 → 0; x' ≤ Sw − W; b ≤ Sh − 5.
        let fr = popup_text(&g, &text, Point::new(100, 0), false, (800, 600)).unwrap();
        assert_eq!(fr.rect.0.x, 100);
        let fr = popup_text(&g, &text, Point::new(5, 0), true, (800, 600)).unwrap();
        assert_eq!(fr.rect.0.x, 0);
        let fr = popup_text(&g, &text, Point::new(799, 700), true, (800, 600)).unwrap();
        assert_eq!(fr.rect, (Point::new(773, 579), Point::new(800, 595)));
    }

    #[test]
    fn horizontal_window() {
        let f = five();
        let opts = TextOpts::Horizontal { s: -7, w: 12 };
        // Pen starts at 3; a glyph is drawn only if pen x > 10 before it;
        // the call stops once pen x > 22.
        assert_eq!(
            place(&f, "abcdef", Point::new(10, 0), 2, opts),
            [('c', 13, 0, 2), ('d', 18, 0, 2)]
        );
        // LF resets pen x to x (not x + s): a glyph at x is not drawn.
        assert_eq!(
            place(
                &f,
                "a\nbc",
                Point::new(10, 50),
                0,
                TextOpts::Horizontal { s: 5, w: 50 }
            ),
            [('a', 15, 50, 0), ('c', 15, 34, 0)]
        );
        assert_eq!(opts.mode(), 5);
        // Spec vector: x 0, s 0, w 7: `a` not drawn (pen 0 is not > 0),
        // `b` at 5, stop before `c` (pen 10 > 7).
        let w7 = TextOpts::Horizontal { s: 0, w: 7 };
        assert_eq!(place(&f, "abc", Point::new(0, 9), 0, w7), [('b', 5, 9, 0)]);
        // The stop test runs before a `LF` and a `ÿc` code too: without it
        // the `LF` would reset the pen and `e` would draw at 5.
        assert_eq!(
            place(&f, "ab\nde", Point::new(0, 9), 0, w7),
            [('b', 5, 9, 0)]
        );
        assert_eq!(
            place(&f, "abÿc1\nde", Point::new(0, 9), 0, w7),
            [('b', 5, 9, 0)]
        );
    }

    #[test]
    fn vertical_window_and_mode_variants() {
        let f = five();
        let g = GlyphLookup::new(&f);
        let at = Point::new(0, 100);
        let style = TextStyle::default();
        let v = |skip| {
            OriginalText.place(
                &g,
                &u("ab"),
                at,
                style,
                &TextOpts::Vertical { skip, lines: 4 },
            )
        };
        assert_eq!(v(-1), Ok(Vec::new()));
        // skip ≥ 0: the glyphs from the pen (their rows are the sprite's,
        // `world_view::ui_bind::vertical_rows`).
        assert_eq!(v(0).unwrap().len(), 2);
        let vert = TextOpts::Vertical { skip: 0, lines: 4 };
        // `ÿc` + 1 unit skipped and no color (k stays 0 whatever the
        // style); `ÿ` + another unit skips 2; `LF` back to x, one line up.
        assert_eq!(
            place(&f, "aÿc1bÿxc\nd", at, 3, vert),
            [
                ('a', 0, 100, 0),
                ('b', 5, 100, 0),
                ('c', 10, 100, 0),
                ('d', 0, 84, 0)
            ]
        );
        // `ÿm` (a MonsterIndicators cel) is not drawn by D2RUST: an error.
        assert!(matches!(
            OriginalText.place(&g, &u("aÿm1"), at, style, &vert),
            Err(TextError::Unspecified(_))
        ));
        let mode = TextOpts::Draw {
            centered: false,
            block_w: None,
            mode: 3,
        };
        assert_eq!(mode.mode(), 3);
        assert_eq!(TextOpts::default().mode(), 5);
    }

    fn wrapped(f: &FontTable, text: &str, max: i32) -> Vec<String> {
        let t = u(text);
        wrap(&GlyphLookup::new(f), &t, max)
            .unwrap()
            .into_iter()
            .map(s)
            .collect()
    }

    #[test]
    fn word_wrap() {
        let f = five();
        assert_eq!(wrapped(&f, "aa bb", 15), ["aa ", "bb"]);
        // Fits: one line, LF kept (no forced break).
        assert_eq!(wrapped(&f, "a\nb", 15), ["a\nb"]);
        // Trailing white space stays; the last span counts the NUL.
        assert_eq!(wrapped(&f, "ab cd ef", 30), ["ab cd ", "ef"]);
        // A leading white-space unit is dropped.
        assert_eq!(wrapped(&f, "ab  cd", 15), ["ab ", "cd"]);
        // Hard break inside a word.
        assert_eq!(wrapped(&f, "abcdefgh", 12), ["ab", "cd", "ef", "gh"]);
        // The break after `ab ` leaves e = s: hard break `␣c` minus its
        // first unit.
        assert_eq!(wrapped(&f, "ab cd", 10), ["ab", "c", "d"]);
    }

    #[test]
    fn word_wrap_empty_lines() {
        let f = five();
        // Width C stops at the NUL: the span ending at L measures as the one
        // ending at L − 1, so no empty third line here.
        assert_eq!(wrapped(&f, "ab  cd", 15), ["ab ", "cd"]);
        let g = GlyphLookup::new(&f);
        let t = u("ab  cd");
        assert_eq!(width_c(&g, &t, 4, 3), Ok(10));
        // (a) The rest is one white-space unit, dropped: an empty line.
        assert_eq!(wrapped(&f, "ab ", 10), ["ab", ""]);
        // (b) A hard break leaves s = L (the last unit alone is wider than
        // M): the next round copies just the NUL.
        let wide = font(10, 5, &[('W', 20)]);
        assert_eq!(wrapped(&wide, "abW", 15), ["ab", "W", ""]);
        assert_eq!(wrapped(&f, "abc", 3), ["a", "b", "c", ""]);
        // Width C skips `ÿc0`–`ÿc6` when deciding the fit.
        assert_eq!(wrapped(&f, "ÿc1ab", 10), ["ÿc1ab"]);
        assert_eq!(wrapped(&f, "ÿc9ab", 10), ["ÿc", "9a", "b"]);
    }

    #[test]
    fn text_call_carries_no_clip() {
        // CG2: the options are the call kind and its arguments only.
        assert_eq!(
            TextOpts::default(),
            TextOpts::Draw {
                centered: false,
                block_w: None,
                mode: 5
            }
        );
        // A pen far outside the frame still places every glyph: pixel
        // clipping is the cel draw's (the item clip, the frame).
        assert_eq!(
            place(&five(), "ab", Point::new(-400, 900), 0, TextOpts::default()),
            [('a', -400, 900, 0), ('b', -395, 900, 0)]
        );
    }

    #[test]
    fn glyphs_resolve_code_to_record_to_frame() {
        let mut f = font16();
        f.glyphs[0x41].frame = 3;
        let out = layout_text(
            &f,
            &[0x41, 0x20AC],
            Point::new(10, 20),
            TextStyle { font: 1, color: 4 },
            &TextOpts::default(),
            &OriginalText,
        )
        .unwrap();
        assert_eq!(
            out,
            [
                GlyphDraw {
                    code: 0x41,
                    record: 0x41,
                    frame: 3,
                    at: Point::new(10, 20),
                    color: 4
                },
                GlyphDraw {
                    code: 0x20AC,
                    record: 0,
                    frame: 0,
                    at: Point::new(22, 20),
                    color: 4
                },
            ]
        );
    }

    #[test]
    fn missing_record_is_an_error_not_a_fallback_glyph() {
        let mut f = five();
        f.glyphs.truncate(0x43);
        let err = layout_text(
            &f,
            &[0x41, 0x43],
            Point::new(0, 0),
            TextStyle::default(),
            &TextOpts::default(),
            &OriginalText,
        )
        .unwrap_err();
        assert_eq!(
            err,
            TextError::MissingGlyph {
                code: 0x43,
                record: 0x43
            }
        );
    }

    #[test]
    fn text_input_caret_blinks_after_the_text() {
        let f = five();
        let g = GlyphLookup::new(&f);
        let mut t = super::TextInput::new(
            super::WidgetId(1),
            Rect::new(10, 20, 50, 10),
            TextStyle::default(),
            8,
        );
        // Blink: on when focused and tick / 1000 is odd.
        assert!(!super::TextInput::caret_visible(true, 999));
        assert!(super::TextInput::caret_visible(true, 1000));
        assert!(!super::TextInput::caret_visible(true, 2000));
        assert!(!super::TextInput::caret_visible(false, 1000));
        let caret = |t: &super::TextInput, tick| {
            let mut out = Vec::new();
            t.draw_caret(&g, true, tick, &mut out).unwrap();
            out
        };
        assert!(caret(&t, 2500).is_empty());
        // Empty text: the caret at the text origin.
        let at = |out: Vec<UiDraw>| match &out[..] {
            [UiDraw::Text(r)] => (r.text.clone(), r.at),
            _ => panic!("one text"),
        };
        assert_eq!(at(caret(&t, 1500)), (vec![0x5F], Point::new(10, 20)));
        t.insert(u16::from(b'a'));
        t.insert(u16::from(b'b'));
        assert_eq!(at(caret(&t, 1500)), (vec![0x5F], Point::new(20, 20)));
        // Line fit: width(line) + wc <= inner width.
        assert_eq!(super::TextInput::fits(&g, &u("abc"), 20), Ok(true));
        assert_eq!(super::TextInput::fits(&g, &u("abc"), 19), Ok(false));
    }
}

#[test]
fn root_mirrors_the_original_ui_flags() {
    let log = Log::default();
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    for id in [1, 2, 40] {
        root.add(TestPanel::boxed(id, FRAME, UiResponse::Ignored, &log))
            .unwrap();
    }
    root.open(PanelId(40)).unwrap();
    let mut states = states::UiStates::new().unwrap();
    states.force(2, true);
    root.sync_states(&states);
    assert_eq!(root.open_panels(), vec![PanelId(2), PanelId(40)]);
    states.force(2, false);
    states.force(1, true);
    root.sync_states(&states);
    assert_eq!(root.open_panels(), vec![PanelId(1), PanelId(40)]);
}

#[test]
fn button_image_changes_only_with_pressed() {
    let up = ImageRef { file: 7, frame: 10 };
    let down = ImageRef { file: 7, frame: 11 };
    let mut b = Button {
        id: WidgetId(1),
        rect: Rect::new(0, 0, 10, 10),
        image: Some(up),
        pressed_image: Some(down),
        pressed: false,
    };
    assert_eq!(b.current_image(), Some(up));
    b.pressed = true;
    assert_eq!(b.current_image(), Some(down));
    let mut out = Vec::new();
    b.draw(&mut out);
    assert!(matches!(&out[..], [UiDraw::Image(r)] if r.image == down));
    // No pressed frame: the image stays.
    b.pressed_image = None;
    assert_eq!(b.current_image(), Some(up));
}

#[test]
fn wheel_scrolls_zero_rows() {
    let mut s = ScrollList::new(WidgetId(1), Rect::new(0, 0, 100, 50), 10).unwrap();
    s.set_len(20);
    s.wheel(120);
    s.wheel(-120);
    s.wheel(1200);
    assert_eq!(s.first(), 0);
    s.scroll(3);
    s.wheel(-120);
    assert_eq!(s.first(), 3);
}

#[test]
fn cell_grid_inventory_geometry() {
    // Inventory 10 × 4 cells of 29 × 29 at (100, 200).
    let g = CellGrid::new(WidgetId(1), Point::new(100, 200), 10, 4, 29, 29).unwrap();
    assert_eq!(g.mouse_cell(Point::new(129, 229)), (1, 1));
    // Left of the grid: the unsigned wrap gives a huge column.
    assert!(g.mouse_cell(Point::new(99, 229)).0 > 1000);
    // Footprint: cells whose top-left corner is outside the clip are cut.
    assert_eq!(
        g.footprint(0, 0, 2, 1, 800, 600),
        vec![Rect::new(100, 200, 29, 29), Rect::new(129, 200, 29, 29)]
    );
    assert_eq!(
        g.footprint(0, 0, 2, 1, 129, 600),
        vec![Rect::new(100, 200, 29, 29)]
    );
    // Hover anchor of a 2 × 3 item at (1, 0).
    assert_eq!(g.hover_anchor(1, 0, 2, 3), (100 + 29 + 29, 200, 200 + 87));
    // Item cel point: (x, top + frame height).
    assert_eq!(g.item_draw_point(2, 1, 58), Point::new(158, 229 + 58));
    // Cursor cell: 1 × 1 at the mouse cell.
    assert_eq!(
        g.cursor_cell(Point::new(129, 229), 1, 1, 28, 28),
        Some((1, 1))
    );
    // 2 × 2 (even): c = ((gw >> 2) - left + mx) / cellW - 1.
    // ((56 >> 2) - 100 + 160) / 29 = 74 / 29 = 2; minus 1 = 1.
    assert_eq!(
        g.cursor_cell(Point::new(160, 260), 2, 2, 56, 56),
        Some((1, 1))
    );
    // 3 wide at the left edge: 0 - 1 clamps to 0.
    assert_eq!(
        g.cursor_cell(Point::new(100, 200), 3, 1, 84, 28),
        Some((0, 0))
    );
    // 2 wide over the right edge: (14 - 100 + 389) / 29 = 10 - 1 = 9; 9 + 2 > 10.
    assert_eq!(g.cursor_cell(Point::new(389, 200), 2, 1, 56, 28), None);
    // The drop cell keeps the overflowing cell: (9, 0).
    assert_eq!(g.drop_cell(Point::new(389, 200), 2, 1, 56, 28), (9, 0));
    // Full-height item: r = gridY >> 1 = 2, minus 2 = 0.
    assert_eq!(
        g.cursor_cell(Point::new(100, 300), 1, 4, 28, 112),
        Some((0, 0))
    );
}

#[test]
fn from_frame_is_the_inverse_of_to_frame() {
    // 1700 × 1300: scale 2, left bar 50, top bar 50.
    let p = Presentation::new(1700, 1300).unwrap();
    assert_eq!(p.from_frame(300, 77), (650, 204));
    let (x, y) = p.from_frame(300, 77);
    assert_eq!(p.to_frame(x, y), FramePos::Inside(Point::new(300, 77)));
}

/// A [`TestPanel`] drawn right before another panel.
struct BeforePanel {
    inner: TestPanel,
    before: PanelId,
}

impl Panel for BeforePanel {
    fn id(&self) -> PanelId {
        self.inner.id()
    }
    fn rect(&self) -> Rect {
        self.inner.rect()
    }
    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        self.inner.draw(ctx, out);
    }
    fn draw_before(&self) -> Option<PanelId> {
        Some(self.before)
    }
    fn hit(&self, p: Point) -> Option<WidgetId> {
        self.inner.hit(p)
    }
    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        self.inner.event(e, ctx)
    }
}

/// `Panel::draw_before`: an open panel added after another is drawn right
/// before it (the inventory family's cube before the inventory), its own
/// place skipped; with the other closed it draws in place; the routing
/// order stays the add order.
#[test]
fn a_draw_before_panel_draws_ahead_of_its_target_only() {
    let log = Log::default();
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    root.add(TestPanel::boxed(
        1,
        Rect::new(0, 0, 100, 100),
        UiResponse::Ignored,
        &log,
    ))
    .unwrap();
    root.add(TestPanel::boxed(
        2,
        Rect::new(0, 0, 100, 100),
        UiResponse::Ignored,
        &log,
    ))
    .unwrap();
    root.add(Box::new(BeforePanel {
        inner: TestPanel {
            id: PanelId(3),
            rect: Rect::new(0, 0, 100, 100),
            button: Button {
                id: WidgetId(30),
                rect: Rect::new(0, 0, 10, 10),
                image: Some(ImageRef { file: 3, frame: 0 }),
                pressed_image: None,
                pressed: false,
            },
            answer: UiResponse::Consumed,
            log: log.clone(),
        },
        before: PanelId(2),
    }))
    .unwrap();
    let files = |root: &UiRoot| -> Vec<u32> {
        let mut out: Vec<UiDraw> = Vec::new();
        with_ctx(|ctx| root.draw(ctx, &mut out));
        out.iter()
            .filter_map(|d| match d {
                UiDraw::Image(i) => Some(i.image.file),
                _ => None,
            })
            .collect()
    };
    for p in 1..=3 {
        root.open(PanelId(p)).unwrap();
    }
    assert_eq!(files(&root), [1, 3, 2]);
    // Routing stays the add order: panel 3 (added last) takes the press.
    with_ctx(|ctx| root.dispatch(press(5, 5), ctx));
    assert_eq!(log.borrow().first().map(|e| e.0), Some(PanelId(3)));
    root.close(PanelId(2)).unwrap();
    assert_eq!(files(&root), [1, 3]);
}

// The root lists the open panels' ids top-most first. A higher panel
// hides an id only where it would take the click (a widget of it or an
// element of its own is there); a screen-wide panel with nothing to hit
// there hides nothing, and a closed panel lists nothing.
#[test]
fn ui_ids_are_listed_top_most_first_unless_a_higher_panel_takes_the_point() {
    let (mut root, log) = overlapping(UiResponse::Ignored, UiResponse::Ignored);
    root.add(Box::new(ListPanel {
        id: PanelId(5),
        answers: Default::default(),
    }))
    .unwrap();
    root.open(PanelId(5)).unwrap();
    let w = ClientWorld::default();
    let ctx = UiCtx {
        tick: 0,
        world: &w,
        strings: &NoStrings,
    };
    let ids = |root: &UiRoot| -> Vec<String> {
        root.ui_elements(&ctx).into_iter().map(|e| e.id).collect()
    };
    assert_eq!(ids(&root), ["test.button(2)", "test.button(1)"]);
    // A panel whose button sits on panel 1's hides it.
    root.add(TestPanel::boxed(
        3,
        Rect::new(0, 0, 50, 50),
        UiResponse::Consumed,
        &log,
    ))
    .unwrap();
    root.open(PanelId(3)).unwrap();
    assert_eq!(ids(&root), ["test.button(3)", "test.button(2)"]);
    root.close(PanelId(3)).unwrap();
    assert_eq!(ids(&root), ["test.button(2)", "test.button(1)"]);
}
