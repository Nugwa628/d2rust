//! Mutation-testing gaps of the UI core.

/// `NoStrings` is the lookup with no strings: every key is absent.
#[test]
fn no_strings_has_no_string() {
    use d2_client::ui::panel::{NoStrings, StringLookup};

    assert_eq!(NoStrings.get("x"), None);
    assert_eq!(NoStrings.get(""), None);
}

/// An intent a panel answers is queued at the root, and
/// `take_intents` hands it over once, oldest first.
#[test]
fn take_intents_drains_the_queued_intents() {
    use d2_client::bridge::world::ClientWorld;
    use d2_client::ui::panel::{
        ClientIntent, NoStrings, Panel, PanelId, PointerButton, UiCtx, UiEvent, UiResponse,
        WidgetId,
    };
    use d2_client::ui::{NoPanelRules, Point, Rect, UiDrawSink, UiRoot};

    struct Asker;
    impl Panel for Asker {
        fn id(&self) -> PanelId {
            PanelId(1)
        }
        fn rect(&self) -> Rect {
            Rect::new(0, 0, 10, 10)
        }
        fn draw(&self, _: &UiCtx, _: &mut dyn UiDrawSink) {}
        fn hit(&self, _: Point) -> Option<WidgetId> {
            None
        }
        fn event(&mut self, e: UiEvent, _: &UiCtx) -> UiResponse {
            match e {
                UiEvent::Press { at, .. } => {
                    UiResponse::Intent(ClientIntent(vec![0x6B, at.x as u8]))
                }
                _ => UiResponse::Ignored,
            }
        }
    }

    let world = ClientWorld::default();
    let ctx = UiCtx {
        tick: 0,
        world: &world,
        strings: &NoStrings,
    };
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    root.add(Box::new(Asker)).unwrap();
    root.open(PanelId(1)).unwrap();
    for x in [1, 2] {
        let press = UiEvent::Press {
            button: PointerButton::Left,
            at: Point::new(x, 1),
        };
        root.dispatch(press, &ctx);
    }
    let want = vec![ClientIntent(vec![0x6B, 1]), ClientIntent(vec![0x6B, 2])];
    assert_eq!(root.take_intents(), want);
    assert!(root.take_intents().is_empty());
}

/// Widgets: plain structs with integer rects; an image widget answers
/// the rect it was given, a grid its column and row counts.
#[test]
fn widget_rects_and_grid_size() {
    use d2_client::ui::panel::WidgetId;
    use d2_client::ui::widget::{CellGrid, FrameImage, Widget};
    use d2_client::ui::{ImageRef, Point, Rect};

    let rect = Rect::new(3, 4, 5, 6);
    let img = FrameImage {
        id: WidgetId(1),
        rect,
        image: ImageRef { file: 0, frame: 0 },
    };
    assert_eq!(img.rect(), rect);
    assert_eq!(img.hit(Point::new(3, 4)), Some(WidgetId(1)));
    let grid = CellGrid::new(WidgetId(2), Point::new(0, 0), 10, 4, 29, 29).unwrap();
    assert_eq!((grid.cols(), grid.rows()), (10, 4));
}

/// Grid of cells: a cell past the last column or row has no rect.
#[test]
fn cell_rect_outside_the_grid_is_none() {
    use d2_client::ui::panel::WidgetId;
    use d2_client::ui::widget::{Cell, CellGrid};
    use d2_client::ui::{Point, Rect};

    let grid = CellGrid::new(WidgetId(2), Point::new(10, 20), 3, 2, 4, 5).unwrap();
    assert_eq!(
        grid.cell_rect(Cell { col: 2, row: 1 }),
        Some(Rect::new(18, 25, 4, 5))
    );
    assert_eq!(grid.cell_rect(Cell { col: 3, row: 0 }), None);
    assert_eq!(grid.cell_rect(Cell { col: 0, row: 2 }), None);
}

/// Scroll list: its length is what the owner set.
#[test]
fn scroll_list_length() {
    use d2_client::ui::panel::WidgetId;
    use d2_client::ui::widget::ScrollList;
    use d2_client::ui::Rect;

    let mut list = ScrollList::new(WidgetId(3), Rect::new(0, 0, 10, 30), 10).unwrap();
    assert!(list.is_empty());
    assert_eq!(list.len(), 0);
    list.set_len(5);
    assert!(!list.is_empty());
    assert_eq!(list.len(), 5);
}

/// Scroll list: the row under a point counts from the list's top
/// edge, and a partly shown row below the last whole row is no row.
#[test]
fn scroll_list_row_at() {
    use d2_client::ui::panel::WidgetId;
    use d2_client::ui::widget::{ScrollList, Widget};
    use d2_client::ui::{Point, Rect};

    let rect = Rect::new(5, 40, 10, 25);
    let mut list = ScrollList::new(WidgetId(3), rect, 10).unwrap();
    assert_eq!(list.rect(), rect);
    list.set_len(10);
    assert_eq!(list.rows_visible(), 2);
    assert_eq!(list.row_at(Point::new(5, 40)), Some(0));
    assert_eq!(list.row_at(Point::new(5, 59)), Some(1));
    assert_eq!(list.row_at(Point::new(5, 62)), None);
}

/// Text input: it answers its rect and draws its text at its origin.
#[test]
fn text_input_rect_and_draw() {
    use d2_client::ui::panel::WidgetId;
    use d2_client::ui::text::TextOpts;
    use d2_client::ui::widget::{TextInput, Widget};
    use d2_client::ui::{Rect, TextRequest, TextStyle, UiDraw, FRAME};

    let rect = Rect::new(7, 8, 100, 12);
    let mut input = TextInput::new(WidgetId(4), rect, TextStyle::default(), 8);
    input.insert(0x41);
    input.insert(0x42);
    assert_eq!(input.rect(), rect);
    let mut out: Vec<UiDraw> = Vec::new();
    input.draw(&mut out);
    assert_eq!(
        out,
        vec![UiDraw::Text(TextRequest {
            text: vec![0x41, 0x42],
            at: rect.origin(),
            style: TextStyle::default(),
            opts: TextOpts::default(),
            clip: FRAME,
        })]
    );
}
