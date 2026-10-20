//! Property tests on the UI core: a `UiRoot` over arbitrary panel trees
//! (rects anywhere in i32, widgets, scripted panel rules) under random
//! open/close/toggle/event sequences; the widgets' integer hit tests; the
//! window → frame mapping. Expected results come from a model of the UI
//! (fixed order, top-most first dispatch, bottom-most first draw, only the
//! root queues intents) and of the presentation (inverse presentation scale, clamp).

use crate::prop_support;

use std::sync::{Arc, Mutex};

use d2_client::bridge::world::ClientWorld;
use d2_client::ui::widget::{CellGrid, ScrollList, Widget};
use d2_client::ui::UiRoot;
use d2_client::ui::{
    ClientIntent, FramePos, ImageRef, ImageRequest, NoStrings, Panel, PanelId, PanelRules, Point,
    PointerButton, Presentation, Rect, Routed, UiCtx, UiDraw, UiDrawSink, UiError, UiEvent,
    UiResponse, WidgetId, FRAME, FRAME_H, FRAME_W,
};
use proptest::prelude::*;

use prop_support::{bounded, config};

/// Half-open containment in i64 (the model of `Rect::contains`).
fn contains(r: Rect, p: Point) -> bool {
    let (x, y) = (i64::from(p.x), i64::from(p.y));
    x >= i64::from(r.x)
        && x < i64::from(r.x) + i64::from(r.w)
        && y >= i64::from(r.y)
        && y < i64::from(r.y) + i64::from(r.h)
}

/// Coordinates: mostly in and around the frame, sometimes at the i32
/// edges.
fn coord() -> impl Strategy<Value = i32> {
    prop_oneof![
        6 => -50i32..900,
        1 => any::<i32>(),
        1 => prop_oneof![Just(i32::MIN), Just(i32::MAX), Just(i32::MAX - 1), Just(i32::MIN + 1)],
    ]
}

/// Window pixels anywhere in i64, edges included (the cursor edge
/// floors any float, `-inf` and `inf` included).
fn wide() -> impl Strategy<Value = i64> {
    prop_oneof![
        3 => any::<i64>(),
        1 => prop_oneof![Just(i64::MIN), Just(i64::MAX), Just(i64::MIN + 1), Just(i64::MAX - 1)],
    ]
}

fn size() -> impl Strategy<Value = u16> {
    prop_oneof![4 => 0u16..400, 1 => any::<u16>()]
}

fn point() -> impl Strategy<Value = Point> {
    (coord(), coord()).prop_map(|(x, y)| Point::new(x, y))
}

fn rect() -> impl Strategy<Value = Rect> {
    (coord(), coord(), size(), size()).prop_map(|(x, y, w, h)| Rect::new(x, y, w, h))
}

/// A test panel: a rect, widget rects (hit: the last one containing the
/// point), and a fixed answer to every event it is offered.
struct TestPanel {
    id: PanelId,
    rect: Rect,
    widgets: Vec<Rect>,
    /// 0 ignored, 1 consumed, 2 intent `[id]`.
    answer: u8,
    seen: Arc<Mutex<Vec<(PanelId, UiEvent)>>>,
}

impl Panel for TestPanel {
    fn id(&self) -> PanelId {
        self.id
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn draw(&self, _: &UiCtx, out: &mut dyn UiDrawSink) {
        out.push(UiDraw::Image(ImageRequest {
            image: ImageRef {
                file: u32::from(self.id.0),
                frame: 0,
            },
            at: self.rect.origin(),
            clip: FRAME,
            look: d2_client::ui::CelLook::PLAIN,
            call: d2_client::ui::draw::CelCall::Draw,
        }));
    }
    fn hit(&self, p: Point) -> Option<WidgetId> {
        self.widgets
            .iter()
            .rposition(|r| r.contains(p))
            .map(|i| WidgetId(i as u16))
    }
    fn event(&mut self, e: UiEvent, _: &UiCtx) -> UiResponse {
        self.seen.lock().unwrap().push((self.id, e));
        match self.answer {
            0 => UiResponse::Ignored,
            1 => UiResponse::Consumed,
            _ => UiResponse::Intent(ClientIntent(vec![self.id.0 as u8])),
        }
    }
}

/// Panel rules that answer from a script (cycled): ids may be unknown or
/// the opening panel itself.
struct Scripted {
    answers: Vec<Vec<PanelId>>,
    next: usize,
}

impl PanelRules for Scripted {
    fn on_open(&mut self, _: PanelId, _: &[PanelId]) -> Vec<PanelId> {
        if self.answers.is_empty() {
            return Vec::new();
        }
        let a = self.answers[self.next % self.answers.len()].clone();
        self.next += 1;
        a
    }
}

#[derive(Debug, Clone)]
struct PanelSpec {
    id: u16,
    rect: Rect,
    widgets: Vec<Rect>,
    answer: u8,
}

fn panel_spec() -> impl Strategy<Value = PanelSpec> {
    (
        0u16..10,
        rect(),
        proptest::collection::vec(rect(), 0..4),
        0u8..3,
    )
        .prop_map(|(id, rect, widgets, answer)| PanelSpec {
            id,
            rect,
            widgets,
            answer,
        })
}

fn event() -> impl Strategy<Value = UiEvent> {
    let button = prop_oneof![
        Just(PointerButton::Left),
        Just(PointerButton::Right),
        Just(PointerButton::Middle)
    ];
    prop_oneof![
        point().prop_map(UiEvent::CursorMoved),
        Just(UiEvent::CursorLeft),
        (button.clone(), point()).prop_map(|(button, at)| UiEvent::Press { button, at }),
        (button, point()).prop_map(|(button, at)| UiEvent::Release { button, at }),
        (any::<i32>(), point()).prop_map(|(steps, at)| UiEvent::Wheel { steps, at }),
        any::<u16>().prop_map(UiEvent::Char),
        any::<u16>().prop_map(|a| UiEvent::Action(d2_client::ui::ActionId(a))),
    ]
}

#[derive(Debug, Clone)]
enum Op {
    Open(u16),
    Close(u16),
    Toggle(u16),
    Event(UiEvent),
    Hit(Point),
    Take,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        2 => (0u16..12).prop_map(Op::Open),
        1 => (0u16..12).prop_map(Op::Close),
        1 => (0u16..12).prop_map(Op::Toggle),
        3 => event().prop_map(Op::Event),
        2 => point().prop_map(Op::Hit),
        1 => Just(Op::Take),
    ]
}

/// The model of the root: panels in add order with their open state.
struct Model {
    panels: Vec<PanelSpec>,
    open: Vec<bool>,
    cursor: Option<Point>,
    intents: Vec<ClientIntent>,
    script: Vec<Vec<PanelId>>,
    next: usize,
}

impl Model {
    fn index(&self, id: u16) -> Option<usize> {
        self.panels.iter().position(|p| p.id == id)
    }

    fn open_ids(&self) -> Vec<PanelId> {
        self.panels
            .iter()
            .zip(&self.open)
            .filter(|(_, o)| **o)
            .map(|(p, _)| PanelId(p.id))
            .collect()
    }

    fn open(&mut self, id: u16) -> Result<(), UiError> {
        let i = self.index(id).ok_or(UiError::UnknownPanel(PanelId(id)))?;
        if self.open[i] {
            return Ok(());
        }
        let close = if self.script.is_empty() {
            Vec::new()
        } else {
            let a = self.script[self.next % self.script.len()].clone();
            self.next += 1;
            a
        };
        let mut idx = Vec::new();
        for c in close {
            if c.0 == id {
                return Err(UiError::RuleClosedOpening { closed: c });
            }
            idx.push(self.index(c.0).ok_or(UiError::UnknownPanel(c))?);
        }
        for c in idx {
            self.open[c] = false;
        }
        self.open[i] = true;
        Ok(())
    }

    fn hit(&self, p: Point) -> Option<(PanelId, Option<WidgetId>)> {
        (0..self.panels.len()).rev().find_map(|i| {
            let s = &self.panels[i];
            (self.open[i] && contains(s.rect, p)).then(|| {
                let w = s
                    .widgets
                    .iter()
                    .rposition(|r| contains(*r, p))
                    .map(|w| WidgetId(w as u16));
                (PanelId(s.id), w)
            })
        })
    }

    /// Expected route and the panels offered the event, top-most first.
    fn dispatch(&mut self, e: UiEvent) -> (Routed, Vec<PanelId>) {
        match e {
            UiEvent::CursorMoved(p) => self.cursor = Some(p),
            UiEvent::CursorLeft => self.cursor = None,
            _ => {}
        }
        let mut offered = Vec::new();
        for i in (0..self.panels.len()).rev() {
            let s = &self.panels[i];
            if !self.open[i] || e.at().is_some_and(|p| !contains(s.rect, p)) {
                continue;
            }
            offered.push(PanelId(s.id));
            match s.answer {
                0 => continue,
                1 => {}
                _ => self.intents.push(ClientIntent(vec![s.id as u8])),
            }
            return (Routed::Panel(PanelId(s.id)), offered);
        }
        (Routed::Unhandled, offered)
    }
}

struct DrawLog(Vec<u32>);

impl UiDrawSink for DrawLog {
    fn push(&mut self, d: UiDraw) {
        if let UiDraw::Image(i) = d {
            self.0.push(i.image.file);
        }
    }
}

fn run(specs: Vec<PanelSpec>, script: Vec<Vec<u16>>, ops: Vec<Op>) {
    let script: Vec<Vec<PanelId>> = script
        .into_iter()
        .map(|v| v.into_iter().map(PanelId).collect())
        .collect();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut root = UiRoot::new(Box::new(Scripted {
        answers: script.clone(),
        next: 0,
    }));
    let mut model = Model {
        panels: Vec::new(),
        open: Vec::new(),
        cursor: None,
        intents: Vec::new(),
        script,
        next: 0,
    };
    for s in specs {
        let dup = model.index(s.id).is_some();
        let got = root.add(Box::new(TestPanel {
            id: PanelId(s.id),
            rect: s.rect,
            widgets: s.widgets.clone(),
            answer: s.answer,
            seen: seen.clone(),
        }));
        if dup {
            assert_eq!(got, Err(UiError::DuplicatePanel(PanelId(s.id))));
        } else {
            assert_eq!(got, Ok(()));
            model.panels.push(s);
            model.open.push(false);
        }
    }
    let world = ClientWorld::default();
    let ctx = UiCtx {
        tick: 0,
        world: &world,
        strings: &NoStrings,
    };
    for op in ops {
        match op {
            Op::Open(id) => {
                let want = model.open(id);
                assert_eq!(root.open(PanelId(id)), want);
            }
            Op::Close(id) => {
                let want = match model.index(id) {
                    Some(i) => {
                        model.open[i] = false;
                        Ok(())
                    }
                    None => Err(UiError::UnknownPanel(PanelId(id))),
                };
                assert_eq!(root.close(PanelId(id)), want);
            }
            Op::Toggle(id) => {
                let want = match model.index(id) {
                    Some(i) if model.open[i] => {
                        model.open[i] = false;
                        Ok(())
                    }
                    Some(_) => model.open(id),
                    None => Err(UiError::UnknownPanel(PanelId(id))),
                };
                assert_eq!(root.toggle(PanelId(id)), want);
            }
            Op::Event(e) => {
                seen.lock().unwrap().clear();
                let (want, offered) = model.dispatch(e);
                assert_eq!(root.dispatch(e, &ctx), want, "{e:?}");
                let got: Vec<PanelId> = seen.lock().unwrap().iter().map(|(p, _)| *p).collect();
                assert_eq!(got, offered, "{e:?}");
            }
            Op::Hit(p) => {
                let got = root.hit(p).map(|h| (h.panel, h.widget));
                assert_eq!(got, model.hit(p), "{p:?}");
            }
            Op::Take => {
                assert_eq!(root.take_intents(), std::mem::take(&mut model.intents));
            }
        }
        assert_eq!(root.open_panels(), model.open_ids());
        assert_eq!(root.intents(), &model.intents[..]);
        let hovered = root.hovered().map(|h| (h.panel, h.widget));
        assert_eq!(hovered, model.cursor.and_then(|p| model.hit(p)));
        // Draw: open panels bottom-most first.
        let mut log = DrawLog(Vec::new());
        root.draw(&ctx, &mut log);
        let want: Vec<u32> = model.open_ids().iter().map(|p| u32::from(p.0)).collect();
        assert_eq!(log.0, want);
    }
}

proptest! {
    #![proptest_config(config(256))]

    /// Arbitrary panel trees and op sequences: the root behaves as the
    /// The UI model (fixed order, top-most first routing over open panels
    /// containing the point, bottom-most first drawing, intents queued
    /// only by the root, nothing changed by a refused open).
    #[test]
    fn root_matches_model(
        specs in proptest::collection::vec(panel_spec(), 0..8),
        script in proptest::collection::vec(proptest::collection::vec(0u16..12, 0..3), 0..4),
        ops in proptest::collection::vec(op(), 0..40),
    ) {
        bounded(move || run(specs, script, ops));
    }

    /// Grid hit tests: a point maps to a cell whose rect contains it, and
    /// every in-range cell's rect lies inside the grid. Grids whose rect
    /// would overflow i32 are refused.
    #[test]
    fn cell_grid(
        origin in point(), cols in size(), rows in size(), cw in size(), ch in size(),
        probes in proptest::collection::vec(point(), 0..16),
        cells in proptest::collection::vec((any::<u16>(), any::<u16>()), 0..16),
    ) {
        bounded(move || {
            let Ok(g) = CellGrid::new(WidgetId(1), origin, cols, rows, cw, ch) else {
                return;
            };
            let r = g.rect();
            assert!(i64::from(r.x) + i64::from(r.w) <= i64::from(i32::MAX) + 1);
            assert!(i64::from(r.y) + i64::from(r.h) <= i64::from(i32::MAX) + 1);
            for p in probes {
                match g.cell_at(p) {
                    Some(c) => {
                        let cr = g.cell_rect(c).expect("cell_at gives an in-range cell");
                        assert!(contains(cr, p), "{p:?} not in {cr:?}");
                    }
                    None => assert!(!contains(r, p)),
                }
            }
            for (col, row) in cells {
                let c = d2_client::ui::widget::Cell { col, row };
                match g.cell_rect(c) {
                    Some(cr) => {
                        assert!(col < g.cols() && row < g.rows());
                        assert!(i64::from(cr.x) >= i64::from(r.x));
                        assert!(i64::from(cr.x) + i64::from(cr.w) <= i64::from(r.x) + i64::from(r.w));
                        assert!(i64::from(cr.y) + i64::from(cr.h) <= i64::from(r.y) + i64::from(r.h));
                        assert_eq!(g.cell_at(cr.origin()), Some(c));
                    }
                    None => assert!(col >= g.cols() || row >= g.rows()),
                }
            }
        });
    }

    /// Scroll lists keep `first` in range under any scroll and length
    /// sequence; a row under a point is a listed row.
    #[test]
    fn scroll_list(
        r in rect(), row_h in size(),
        ops in proptest::collection::vec(
            prop_oneof![
                any::<i64>().prop_map(|s| (0u8, s)),
                prop_oneof![Just(i64::MIN), Just(i64::MAX)].prop_map(|s| (0u8, s)),
                (-5i64..5).prop_map(|s| (0u8, s)),
                any::<u32>().prop_map(|l| (1u8, i64::from(l))),
                (0i64..40).prop_map(|l| (1u8, l)),
            ],
            0..16,
        ),
        probes in proptest::collection::vec(point(), 0..8),
    ) {
        bounded(move || {
            let Ok(mut l) = ScrollList::new(WidgetId(2), r, row_h) else {
                return;
            };
            for (kind, v) in ops {
                if kind == 0 {
                    l.scroll(v);
                } else {
                    l.set_len(v as u32);
                }
                let max_first = l.len().saturating_sub(l.rows_visible());
                assert!(l.first() <= max_first);
                for &p in &probes {
                    if let Some(row) = l.row_at(p) {
                        assert!(row < l.len() && row >= l.first());
                        assert!(contains(l.rect(), p));
                    }
                }
            }
        });
    }

    /// Window → frame: any window pixel maps inside the frame or to
    /// `Outside`, and the window pixel of each frame point maps back to it.
    #[test]
    fn window_to_frame(
        w in prop_oneof![800u32..4000, any::<u32>()],
        h in prop_oneof![600u32..3000, any::<u32>()],
        probes in proptest::collection::vec((wide(), wide()), 0..8),
        near in proptest::collection::vec((-10i64..5000, -10i64..5000), 0..8),
        fp in (0i32..800, 0i32..600),
    ) {
        bounded(move || {
            let Ok(p) = Presentation::new(w, h) else {
                assert!(w / u32::from(FRAME_W) == 0 || h / u32::from(FRAME_H) == 0);
                return;
            };
            assert!(p.scale >= 1);
            assert!(u64::from(p.left) * 2 + u64::from(FRAME_W) * u64::from(p.scale) <= u64::from(w));
            assert!(u64::from(p.top) * 2 + u64::from(FRAME_H) * u64::from(p.scale) <= u64::from(h));
            for (x, y) in probes.into_iter().chain(near) {
                if let FramePos::Inside(q) = p.to_frame(x, y) {
                    assert!(contains(FRAME, q), "{q:?}");
                }
            }
            let s = i64::from(p.scale);
            let (x, y) = (i64::from(p.left) + i64::from(fp.0) * s, i64::from(p.top) + i64::from(fp.1) * s);
            assert_eq!(p.to_frame(x, y), FramePos::Inside(Point::new(fp.0, fp.1)));
            assert_eq!(p.to_frame(x + s - 1, y + s - 1), FramePos::Inside(Point::new(fp.0, fp.1)));
        });
    }

    /// `Rect::contains` agrees with the i64 model for any rect and point.
    #[test]
    fn rect_contains(r in rect(), p in point()) {
        prop_assert_eq!(r.contains(p), contains(r, p));
    }
}

// Minimized failures of the properties above (fixed at the root).

/// `Rect::contains` overflowed i32 for a rect ending past `i32::MAX`.
#[test]
fn regress_rect_edge_past_i32() {
    let r = Rect::new(i32::MAX - 1, 0, 10, 1);
    assert!(r.contains(Point::new(i32::MAX, 0)));
    assert!(!r.contains(Point::new(i32::MAX - 2, 0)));
}

/// A grid whose cells pass `i32::MAX` was accepted; `cell_rect` then
/// overflowed.
#[test]
fn regress_cell_grid_past_i32() {
    let g = CellGrid::new(WidgetId(1), Point::new(i32::MAX - 10, 0), 2, 1, 10, 1);
    assert!(g.is_err());
    let g = CellGrid::new(WidgetId(1), Point::new(i32::MAX - 19, 0), 2, 1, 10, 1).unwrap();
    let c = d2_client::ui::widget::Cell { col: 1, row: 0 };
    assert_eq!(g.cell_rect(c), Some(Rect::new(i32::MAX - 9, 0, 10, 1)));
    assert_eq!(g.cell_at(Point::new(i32::MAX, 0)), Some(c));
}

/// `ScrollList::scroll` overflowed i64 for a step at the i64 edge.
#[test]
fn regress_scroll_by_i64_max() {
    let mut l = ScrollList::new(WidgetId(2), Rect::new(0, 0, 10, 10), 1).unwrap();
    l.set_len(100);
    l.scroll(5);
    l.scroll(i64::MAX);
    assert_eq!(l.first(), 90);
    l.scroll(i64::MIN);
    assert_eq!(l.first(), 0);
}

/// `Presentation::to_frame` overflowed i64 for a cursor at the i64 edge
/// (`window_pixel` floors `-inf` to `i64::MIN`).
#[test]
fn regress_window_pixel_at_i64_min() {
    let p = Presentation::new(1700, 1300).unwrap();
    assert!(p.left > 0 && p.top > 0);
    assert_eq!(p.to_frame(i64::MIN, 0), FramePos::Outside);
    assert_eq!(p.to_frame(0, i64::MIN), FramePos::Outside);
    assert_eq!(p.to_frame(i64::MAX, i64::MAX), FramePos::Outside);
}
