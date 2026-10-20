//! Synthetic registrations of each client hook (seams MS14), and the
//! check that an empty registry leaves D2RUST's UI as it is.

use d2_proto::item_bits::ItemBits;

use super::*;
use crate::bridge::world::{ClientUnit, UnitKey, PLAYER};
use crate::ui::draw::{TextRequest, TextStyle, UiDraw, UiDrawSink};
use crate::ui::geom::{Point, Rect, FRAME};
use crate::ui::item_tip::{color, ItemTips};
use crate::ui::item_tip_build::TipCtx;
use crate::ui::layout::Screen;
use crate::ui::mod_hud::MOD_HUD_PANEL;
use crate::ui::original::hud::HUD_PANEL;
use crate::ui::original::{OriginalUi, UiConfig};
use crate::ui::panel::{Panel, PanelId, UiEvent, UiResponse, WidgetId};
use crate::ui::panels::UiFiles;
use crate::ui::text::TextOpts;
use crate::ui::{NoPanelRules, NoStrings, UiCtx, UiRoot};

/// A HUD line source that shows how many units the client knows.
struct UnitCount;

impl HudLines for UnitCount {
    fn lines(&self, world: &ClientWorld, out: &mut Vec<HudLine>) {
        out.push(HudLine::new(
            format!("units {}", world.units.len()),
            Point::new(20, 40),
        ));
    }
}

/// An item hook that tags caps and turns the top line gold.
struct TagCaps;

impl ItemLines for TagCaps {
    fn lines(&self, item: &ItemBits, lines: &mut Vec<TipLine>) {
        if &item.code == b"cap " {
            lines.push(TipLine::new("mod: a cap", color::GREEN));
            if let Some(top) = lines.first_mut() {
                top.color = color::GOLD;
            }
        }
    }
}

fn world() -> ClientWorld {
    let mut w = ClientWorld::default();
    let key = UnitKey::new(PLAYER, 1);
    w.units.insert(key, ClientUnit::new(key));
    w.local_player = Some(key);
    w
}

/// The play UI's root with `mods`, as `app::ui` builds it.
fn root_with(mods: Option<ClientMods>) -> (OriginalUi, UiRoot) {
    let config = UiConfig {
        screen: Screen::R800,
        expansion_installed: true,
    };
    let mut ui = OriginalUi::new(config, None).unwrap();
    if let Some(m) = mods {
        ui.set_mods(m);
    }
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    ui.install(&mut root).unwrap();
    (ui, root)
}

fn texts(root: &UiRoot, w: &ClientWorld) -> Vec<TextRequest> {
    let ctx = UiCtx {
        tick: 0,
        world: w,
        strings: &NoStrings,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    root.draw(&ctx, &mut out);
    out.into_iter()
        .filter_map(|d| match d {
            UiDraw::Text(t) => Some(t),
            _ => None,
        })
        .collect()
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

// An empty registry (and one with only non-HUD hooks) adds no panel: the
// root and its draws are D2RUST's own.
#[test]
fn no_hud_hook_leaves_the_root_unchanged() {
    let w = world();
    let (_, plain) = root_with(None);
    let (_, empty) = root_with(Some(ClientMods::new()));
    let (_, items_only) = root_with(Some(ClientMods::new().with_item_lines(TagCaps)));
    assert_eq!(plain.open_panels(), empty.open_panels());
    assert_eq!(plain.open_panels(), items_only.open_panels());
    assert!(!plain.open_panels().contains(&MOD_HUD_PANEL));
    assert_eq!(texts(&plain, &w), texts(&empty, &w));
}

// MS14a: a registered HUD line source is drawn every frame, right above
// the HUD, from the client's model.
#[test]
fn hud_lines_are_drawn_above_the_hud() {
    let w = world();
    let (_, root) = root_with(Some(ClientMods::new().with_hud_lines(UnitCount)));
    let open = root.open_panels();
    let hud = open.iter().position(|&p| p == HUD_PANEL).unwrap();
    assert_eq!(open[hud + 1], MOD_HUD_PANEL);
    let line = texts(&root, &w)
        .into_iter()
        .find(|t| t.text == utf16("units 1"))
        .expect("the mod line is drawn");
    assert_eq!(line.at, Point::new(20, 40));
    assert_eq!((line.style.font, line.style.color), (1, color::WHITE));
    // The panel takes no input.
    assert_ne!(
        root.hit(Point::new(20, 40)).map(|h| h.panel),
        Some(MOD_HUD_PANEL)
    );
}

// MS14a: an item hook sees the decoded item and D2RUST's lines, and its
// changes are the tip's lines; other items keep D2RUST's.
#[test]
fn item_lines_hook_changes_the_hover_tip() {
    let plain = crate::ui::item_tip::tests::tips();
    let mut modded: ItemTips = plain.clone();
    modded.mods = ClientMods::new().with_item_lines(TagCaps);
    let cap = ItemBits {
        code: *b"cap ",
        quality: 2,
        ..ItemBits::default()
    };
    let ctx = TipCtx::default();
    let before = plain.tip_lines_of(&cap, &ctx);
    let after = modded.tip_lines_of(&cap, &ctx);
    assert!(!before.is_empty());
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(after[0].text, before[0].text);
    assert_eq!(after[0].color, color::GOLD);
    assert_eq!(
        after.last().unwrap(),
        &TipLine::new("mod: a cap", color::GREEN)
    );
    let other = ItemBits {
        code: *b"buc ",
        quality: 2,
        ..ItemBits::default()
    };
    assert_eq!(
        modded.tip_lines_of(&other, &ctx),
        plain.tip_lines_of(&other, &ctx)
    );
}

/// A synthetic mod panel: draws one text line naming itself and the id of
/// its art file.
struct TestPanel {
    id: PanelId,
    file: Option<u32>,
}

impl Panel for TestPanel {
    fn id(&self) -> PanelId {
        self.id
    }
    fn rect(&self) -> Rect {
        Rect::new(100, 100, 200, 100)
    }
    fn draw(&self, _ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        out.push(UiDraw::Text(TextRequest {
            text: utf16(&format!("panel {:#x} file {:?}", self.id.0, self.file)),
            at: Point::new(110, 120),
            style: TextStyle { font: 1, color: 0 },
            opts: TextOpts::default(),
            clip: FRAME,
        }));
    }
    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }
    fn event(&mut self, _e: UiEvent, _ctx: &UiCtx) -> UiResponse {
        UiResponse::Consumed
    }
}

/// Makes a [`TestPanel`] with `id` that draws `art`.
struct MakePanel {
    id: PanelId,
    art: &'static str,
}

impl PanelFactory for MakePanel {
    fn files(&self) -> Vec<String> {
        vec![self.art.to_string()]
    }
    fn make(&self, files: &UiFiles) -> Box<dyn Panel> {
        Box::new(TestPanel {
            id: self.id,
            file: files.id(self.art),
        })
    }
}

fn drawn(root: &UiRoot, w: &ClientWorld, text: &str) -> bool {
    texts(root, w).iter().any(|t| t.text == utf16(text))
}

// MS14b: an added panel goes above the game's panels, below the Esc menu
// and the cursor layer; its art file is registered after the game's.
#[test]
fn added_panel_is_drawn_below_the_esc_menu() {
    let w = world();
    let id = PanelId(MOD_PANEL_IDS);
    let factory = MakePanel { id, art: "mod_box" };
    let (plain_ui, _) = root_with(None);
    let (ui, root) = root_with(Some(
        ClientMods::new().with_panel(PanelPlace::Add { open: true }, factory),
    ));
    let file = ui.files().id("mod_box").expect("registered");
    assert_eq!(file as usize, plain_ui.files().names().len());
    let open = root.open_panels();
    let at = open.iter().position(|&p| p == id).unwrap();
    assert_eq!(open[at + 1], crate::ui::original::TOP_PANEL);
    assert!(drawn(&root, &w, &format!("panel 0x1000 file Some({file})")));
    assert_eq!(root.hit(Point::new(150, 150)).map(|h| h.panel), Some(id));
    // Closed from the start when asked.
    let (_, root) = root_with(Some(ClientMods::new().with_panel(
        PanelPlace::Add { open: false },
        MakePanel { id, art: "mod_box" },
    )));
    assert!(!root.open_panels().contains(&id));
}

// MS14b: a replacement takes the game panel's place and open state.
#[test]
fn replacement_panel_takes_the_game_panels_place() {
    let w = world();
    let (_, plain) = root_with(None);
    let (_, root) = root_with(Some(ClientMods::new().with_panel(
        PanelPlace::Replace,
        MakePanel {
            id: HUD_PANEL,
            art: "mod_hud",
        },
    )));
    assert_eq!(plain.open_panels(), root.open_panels());
    assert!(texts(&root, &w)
        .iter()
        .any(|t| String::from_utf16_lossy(&t.text).starts_with("panel 0x101 file Some(")));
    // A replacement for a panel the game doesn't have is an error.
    let config = UiConfig {
        screen: Screen::R800,
        expansion_installed: true,
    };
    let mut ui = OriginalUi::new(config, None).unwrap();
    ui.set_mods(ClientMods::new().with_panel(
        PanelPlace::Replace,
        MakePanel {
            id: PanelId(0x7FFF),
            art: "x",
        },
    ));
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    assert!(ui.install(&mut root).is_err());
}

/// Hides buckles, renames caps and turns their label blue.
struct CapFilter;

impl ItemNameFilter for CapFilter {
    fn filter(&self, item: &ItemBits, label: &mut ItemLabel) {
        match &item.code {
            b"buc " => label.shown = false,
            b"cap " => {
                label.text.extend(utf16(" *"));
                label.color = color::BLUE;
            }
            _ => {}
        }
    }
}

// MS14b: a label filter renames, recolours and hides ground item labels;
// without one the labels are D2RUST's.
#[test]
fn item_name_filter_changes_ground_labels() {
    let plain = crate::ui::item_tip::tests::tips();
    let mut modded: ItemTips = plain.clone();
    modded.mods = ClientMods::new().with_item_name_filter(CapFilter);
    let ctx = TipCtx::default();
    let item = |code: &[u8; 4]| ItemBits {
        code: *code,
        quality: 2,
        ..ItemBits::default()
    };
    let (name, _) = plain.label_of(&item(b"cap "), &ctx).unwrap();
    let mut tagged = name.clone();
    tagged.extend(utf16(" *"));
    assert_eq!(
        modded.label_of(&item(b"cap "), &ctx),
        Some((tagged, color::BLUE))
    );
    assert!(plain.label_of(&item(b"buc "), &ctx).is_some());
    assert_eq!(modded.label_of(&item(b"buc "), &ctx), None);
}
