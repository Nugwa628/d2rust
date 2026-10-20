//! The play UI's panels wired into a [`UiRoot`]: [`OriginalUi`] owns the
//! 38 UI flags ([`UiStates`], the authority over which panels are open),
//! the loaded panel tables and the panels' state; [`OriginalUi::install`]
//! adds one [`Panel`] adapter per panel to the root, in draw order, with
//! the panel's UI state id as its [`PanelId`] (so [`UiRoot::sync_states`]
//! mirrors the flags). Panels that are not UI states (border, HUD,
//! messages, the cursor layer, a mod game's HUD lines) use ids from 0x100.
//!
//! Event flow (one UI frame, [`crate::world_view::run_ui_with`]): the
//! root routes each event to the adapters, top-most first; an adapter runs
//! the panel module's own press / release rules and parks their
//! [`PanelOutput`]s here. After each event [`OriginalUi::after_event`]
//! applies them in order: an intent is queued on the root (only the root
//! forwards it to the bridge), a `SetUIState` request runs
//! [`UiStates::set`] (gate, flags, open mode) and its effects, and the
//! click sound is queued for the audio side. An event no panel took that
//! is a D2RUST hotkey action (`ToggleInventory`, `ToggleCharacter`,
//! `ToggleSkillTree`, …) becomes `SetUIState(ui, toggle, no cursor jump)`.
//! The root then mirrors the flags.
//!
//! The character panel's name line and stat values need the fonts' widths
//! ([`FontMeasure`], [`OriginalUi::set_fonts`]); parts the client model
//! has no input for yet stay out, each with its reason in [`PENDING`].
//! The bridge's UI outputs (S→C 0x5D, 0x63, 0x77) are applied by
//! [`OriginalUi::apply_output`] (`msg_ui`). A mod game's client hooks
//! (`crate::mods`) reach the panels through [`OriginalUi::set_mods`].
//! Nothing here decides an outcome: the client sends intents and draws.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use d2_formats::font::FontTable;

use super::draw::UiDrawSink;
use super::geom::{Point, Rect};
use super::item_tip;
use super::layout::{LayoutError, PanelKey, RowKind, Screen};
use super::panel::{ActionId, Panel, PanelId, UiCtx, UiEvent, UiResponse, WidgetId};
use super::panels::border::{draw_border_and_ctrlpnl, draw_ctrlpnl_sides};
use super::panels::char_details;
use super::panels::char_inputs;
use super::panels::character::{
    self, CharacterPanel, CharacterView, ResistEffect, STAT_STATPTS, UI_CHARACTER,
};
use super::panels::inv_gold;
use super::panels::inv_items::{InvLayout, ItemsUi};
use super::panels::inventory::{InventoryPanel, UI_INVENTORY};
use super::panels::skilltree::{SkillEntry, SkillTreePanel, SkillTreeView, UI_SKILLTREE};
use super::panels::{
    centered_in, emit_static_draws, no_extra, text, PanelEnv, PanelOutput, PanelTables,
    TextMeasure, UiFiles,
};
use super::root::{Routed, UiError, UiRoot};
use super::skill_tree_ui;
use super::states::{GateEnv, PlayerLife, UiEffect, UiStateError, UiStates};
use super::PointerButton;
use crate::assets::path::FileSource;
use crate::audio::driver::SoundRequest;
use crate::bridge::items::ItemArtRows;
use crate::bridge::msg::ui_npc::DialogCase;
use crate::bridge::output::NpcDialog;
use crate::bridge::world::{ClientWorld, KindData, UnitKey, PLAYER};
use crate::controls::Action;
use crate::mods::{ClientMods, PanelPlace};
use crate::rules::camera::{FrameSize, OpenMode};

/// The border / control panel adapter's id: not a UI state (ids 0–37
/// are), so [`UiRoot::sync_states`] leaves it open.
pub const BORDER_PANEL: PanelId = PanelId(0x100);

/// The panels that cover the world where their rect lies, so the world
/// under them is not hovered: every UI-state panel (ids 0–37: the
/// inventory, character, skill tree, stash, cube, shop, waypoint, quest
/// and message panels, the NPC menu, the game menu), the control panel
/// strip (with the mini panel and a popped belt) and the hireling list.
/// The overlays that only draw over the frame or catch presses anywhere
/// (the gold dialog's adapter, the tips, the messages) do not.
pub fn covers_world(id: PanelId) -> bool {
    usize::from(id.0) < super::layout::UI_STATE_COUNT
        || id == hud::HUD_PANEL
        || id == super::hire_list::HIRE_PANEL
}

/// The Esc-closable states (flag table = 1), in the close-all's order i = 0 …
/// 37.
pub const ESC_CLOSABLE: [u8; 27] = [
    1, 2, 3, 4, 5, 9, 0x0B, 0x0C, 0x0D, 0x0F, 0x10, 0x12, 0x14, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B,
    0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x24, 0x25,
];

/// The states the game menu's open remembers and its close reopens (keep = 1).
pub const GAME_MENU_KEEP: [u8; 6] = [6, 7, 10, 17, 21, 35];

/// The step-10 overlays' adapter (hover tips, then the cursor item), drawn
/// after every other panel: not a UI state, open for good.
pub const TOP_PANEL: PanelId = PanelId(0x113);

/// The click sound: request id 0 (silent), no unit, delay 0, at the sites
/// that play no specific sound (the sites with a constant id 1–6, 15, 16
/// play their own).
pub const CLICK_SOUND_ID: i32 = 0;

/// Panels and inputs not wired, each with the input the client model or
/// the app does not hold yet (M02: named, not guessed).
pub const PENDING: &[(&str, &str)] = &[
    (
        "character shift-spend, popups and the skill-modified damage block (§8.6, §8.9; \
         `panels-2.md` §17 r5–r9)",
        "labels (string ids), the class line, next level, the resist / defense colours and a \
         weapon-only damage / attack-rating block are bound ([`char_feed`]); the skill-specific \
         `descdam` / `descatt` entries are bound too (the `ui-draws-char-desc-*` checks, 46 \
         MATCH on 2026-10-15); the chance-to-hit popup (`panels-2.md` §17 r6) is not: the reference client \
         draws \"Average chance to hit\" and \"<monster>: 95%\" over the attack rating with a \
         monster hovered first, D2RUST nothing (PROVISIONAL REC-269, settled by the draws check \
         `w2-02-char-tohit-popup`, now DIVERGED; it needs the hovered monster's `monstats` row \
         and level). Shift spends in chunks of 32 (`panels.md` §8 r5, REC-268); the language \
         is English (0, `ui/text.md` §1.2)",
    ),
    (
        "inventory gold button press / release and the gold dialog (`panels-2.md` §21 r3–r9)",
        "the gold value and button art are drawn ([`InventoryUi`]); the press / release and \
         the drop dialog (kind 1) are wired in [`gold_dialog`] (D2RUST's own box, REC-103); the \
         press does not play sound 4 (deferred); the stash kinds 3 / 4 are wired (\
         `original_tests.rs::stash_gold_withdraw_and_deposit_send_0x4f`)",
    ),
    (
        "inventory tints (translucency) (`inventory.md` §2–§6)",
        "the equipped and grid item tints and the hover tint are painted by \
         `ItemsUi::draw_tints` as opaque `hudfill` tiles (REC-271: the UI sprite path has no \
         A2 blend); the shooter / quiver and cursor-item tints are not read",
    ),
    (
        "skill tree hover description and the no-points message (§10.5, §10.7)",
        "icons, level numbers, the flag mask, the point spend (C→S 0x3B), the free-points \
         number and the tab / close tool tips are wired ([`skill_tree_ui`], `SkillTreeUi`); \
         the point cost is 1 (`skpoints` formula not evaluated, REC-270); the hover \
         description and the no-points message are not wired",
    ),
    (
        "waypoint menu panel (ui 0x14, §13 r2–r7): row names",
        "the panel is installed and covered (`waypoint_ui`; \
         `app_play_npc.rs::clicking_the_waypoint_walks_there_and_interacts`); the tabs read \
         the client quest flags (`menus.md` §1.4, §13.3); the row text is the `levels` \
         `LevelName` key through the string table (level-name lookup not specified, D2RUST's own)",
    ),
    (
        "hotkeys for other states (escape menu, chat, automap, party; the quest log, ToggleQuests, is \
         bound: `quest_log_ui_tests.rs`)",
        "their panels are not specified (§Open questions 1); the reference key table is \
         `ui/controls.md` (§Open questions 2)",
    ),
];

/// The screen and install facts the play UI reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiConfig {
    /// W × H and the resolution mode (the 800 × 600 frame: [`Screen::R800`]).
    pub screen: Screen,
    /// `d2exp.mpq` present.
    pub expansion_installed: bool,
}

/// One `inventory.bin` `inv` rectangle: left, right
/// (exclusive), top, bottom (inclusive).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InvArea {
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
}

impl InvArea {
    /// The right panel's click area: left ≤ x < right, top ≤ y ≤ bottom (bottom
    /// inclusive).
    pub fn rect(&self) -> Rect {
        let w = u16::try_from(self.right - self.left).unwrap_or(0);
        let h = u16::try_from(self.bottom - self.top + 1).unwrap_or(0);
        Rect::new(self.left, self.top, w, h)
    }
}

/// The `inventory.bin` record of a player class for the page-0 grid (classes
/// 0–4 → 0–4, 5 → 14, 6 → 15) plus 16 at 800 × 600. `None` for a class outside
/// 0–6.
pub fn inventory_record(class: u32, screen: &Screen) -> Option<usize> {
    let r = match class {
        0..=4 => class as usize,
        5 => 14,
        6 => 15,
        _ => return None,
    };
    Some(if screen.res2() { r + 16 } else { r })
}

// The `inventory` table places every grid, equipment box and click area
// for the 800 × 600 layout. In a wider frame the side panels open at the
// frame's edges, so each rectangle moves with its side
// (`Screen::side_dx`: left of the box's middle with the left panels,
// else with the right ones). At 4:3 nothing moves.

/// An `inv` click area at the edges of `s`.
pub fn at_edges_area(s: &Screen, a: InvArea) -> InvArea {
    if (a.left, a.right) == (0, 0) {
        return a;
    }
    let dx = s.side_dx(a.left);
    InvArea {
        left: a.left + dx,
        right: a.right + dx,
        top: a.top + s.panel_dy(),
        bottom: a.bottom + s.panel_dy(),
    }
}

/// A grid and its equipment boxes at the edges of `s`; empty boxes stay.
pub fn at_edges_layout(s: &Screen, mut l: InvLayout) -> InvLayout {
    let g = &mut l.grid;
    if (g.left, g.right) != (0, 0) {
        let dx = s.side_dx(g.left);
        (g.left, g.right) = (g.left + dx, g.right + dx);
        (g.top, g.bottom) = (g.top + s.panel_dy(), g.bottom + s.panel_dy());
    }
    for b in &mut l.equip {
        if (b.left, b.right) != (0, 0) {
            let dx = s.side_dx(b.left);
            (b.left, b.right) = (b.left + dx, b.right + dx);
            (b.top, b.bottom) = (b.top + s.panel_dy(), b.bottom + s.panel_dy());
        }
    }
    l
}

/// The empty-slot picture boxes at the edges of `s`; empty boxes stay.
pub fn at_edges_equip(
    s: &Screen,
    mut e: super::panels::inventory::EquipRects,
) -> super::panels::inventory::EquipRects {
    for b in [
        &mut e.r_arm,
        &mut e.torso,
        &mut e.l_arm,
        &mut e.head,
        &mut e.neck,
        &mut e.r_hand,
        &mut e.l_hand,
        &mut e.belt,
        &mut e.feet,
        &mut e.gloves,
    ] {
        if (b.left, b.right) != (0, 0) {
            let dx = s.side_dx(b.left);
            (b.left, b.right) = (b.left + dx, b.right + dx);
            (b.top, b.bottom) = (b.top + s.panel_dy(), b.bottom + s.panel_dy());
        }
    }
    e
}

/// The model facts a frame of the play UI reads, taken from the
/// client world before each event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Facts {
    /// Local player class, when there is a local player.
    class: Option<u32>,
    player: Option<PlayerLife>,
    /// An expansion game.
    expansion_game: bool,
    /// The local player, for player events.
    local: Option<UnitKey>,
    /// A local player with a room (D2RUST's own: read as a position), for the
    /// waypoint close hook.
    player_room: bool,
    /// The local player has a hireling (its hireling slot is set), for the
    /// hireling key (command 54).
    has_hireling: bool,
    /// An item worn at body location 8 (a belt, item type 0x13), for the
    /// belt key (command 22).
    has_belt: bool,
}

impl Facts {
    fn of(world: &ClientWorld) -> Self {
        let local = world.local().filter(|u| u.key.unit_type == PLAYER);
        Facts {
            class: local.map(|u| u.class),
            // "P alive": type 0, mode neither 0 nor 0x11; flag 0x10000
            // is not in the model (`ClientUnit::is_dead`).
            player: local.map(|u| PlayerLife {
                alive: !u.is_dead(),
                dead: u.mode == 0x11,
            }),
            expansion_game: world.expansion != 0,
            local: local.map(|u| u.key),
            player_room: local.is_some_and(|u| u.position.is_some()),
            has_hireling: local.is_some_and(|u| world.hireling_guid(Some(u.key)) != u32::MAX),
            has_belt: crate::bridge::items::local_items(world)
                .iter()
                .any(|i| i.mode == crate::bridge::items::mode::BODY && i.body == 8),
        }
    }
}

/// State shared by the controller and its panel adapters.
struct Shared {
    tables: PanelTables,
    states: UiStates,
    config: UiConfig,
    /// `inventory.bin` `inv` rectangles by record, when loaded.
    inv_areas: Option<Vec<InvArea>>,
    /// Loaded rectangles stay untransformed for repeated live resizes.
    base_inv_areas: Option<Vec<InvArea>>,
    base_layouts: Option<Vec<InvLayout>>,
    base_equip: Option<Vec<super::panels::inventory::EquipRects>>,
    facts: Facts,
    /// The mouse as the UI last saw it (the cursor jump reads it before a call).
    mouse: Point,
    /// Panel outputs of the event being routed, in order.
    outputs: Vec<PanelOutput>,
    /// An input reset (clear held clicks and keys) was asked for since the host last took
    /// it ([`OriginalUi::take_input_reset`]).
    input_reset: bool,
    /// Clear Screen closed nothing: the automap part is the host's
    /// ([`OriginalUi::take_clear_automap`]).
    clear_automap: bool,
    /// The fonts' glyph widths (character values and name line); none:
    /// no text is drawn.
    fonts: Option<FontMeasure>,
    /// `difficultylevels` `ResistPenalty` by difficulty;
    /// an expansion game draws no values without it.
    resist_penalties: Option<Vec<i32>>,
    /// The character panel's extra tables ([`char_feed`]).
    char_tables: super::char_feed::CharTables,
    /// The control panel overlays' state ([`hud`]).
    hud: hud::HudState,
    /// The skill tree's class skill rows ([`skill_tree_ui`]).
    skill_tree: skill_tree_ui::SkillTreeTables,
    /// The inventory panel's item facts (`inv_items`).
    items: ItemsUi,
    /// The levels' waypoint indexes (`waypoint_ui`).
    waypoint_map: Option<d2_sim::world::waypoints::WaypointMap>,
    /// The waypoint menu S→C 0x63 opened last (`waypoint_ui`).
    waypoint_open: Option<WaypointOpen>,
    /// The Esc game menu's state ([`esc_menu`]).
    esc: esc_menu::EscState,
    /// The ui states the game menu's open closed and reopens at its close.
    esc_kept: Vec<u8>,
    /// The quest log's inputs ([`quest_log_ui`]).
    quest: quest_log_ui::QuestInputs,
    /// The inventory gold button and the drop-gold dialog ([`gold_dialog`]).
    gold: gold_dialog::GoldState,
    /// The screen message list ([`game_messages`]).
    messages: game_messages::GameMessages,
    /// The overhead text bubbles ([`overhead_ui`]).
    bubbles: overhead_ui::Bubbles,
    /// S→C 0x77 0x15 opened the cube since the panel last looked: its close
    /// latch clears ([`cube_ui`]).
    cube_opened: bool,
    /// The transmute animation ([`cube_ui`]).
    cube_anim: std::cell::Cell<super::panels::stash_cube::HoradricAnim>,
    /// The mouse cursor machine, drawn last by
    /// [`TopUi`], and the local copy of the client seed its step draws on
    /// when no shared seed is linked.
    cursor: RefCell<super::cursor::Cursor>,
    cursor_seed: std::cell::Cell<Option<u64>>,
    /// The local player's client seed shared with the weather and sound
    /// draws (one random stream for all three); the cursor
    /// step draws on it when linked ([`OriginalUi::set_client_seed`]).
    client_seed: Option<std::sync::Arc<std::sync::Mutex<crate::audio::driver::ClientSeed>>>,
    /// A cursor step owed by the last cursor draw (run by
    /// [`OriginalUi::cursor_step`]).
    cursor_step_owed: std::cell::Cell<bool>,
    /// The cursor type and frame of the tick whose draw waits for the GPU
    /// after its step ran ([`OriginalUi::hold_cursor_frame`]).
    cursor_hold: std::cell::Cell<Option<(u64, u8, i32)>>,
    /// The last cursor draw was the held item's graphic (the last draw of
    /// the frame), read by [`OriginalUi::cursor_item_drawn`].
    cursor_item_drawn: std::cell::Cell<bool>,
    /// The host clock set by the app ([`OriginalUi::set_host_now`]).
    host_now: std::cell::Cell<Option<u32>>,
    /// The UI's wall clock in live play ([`OriginalUi::set_menu_wall_ms`]):
    /// the Esc menu's pentagrams and the UI timers ([`super::ui_clock`]).
    pub(super) menu_wall_ms: std::cell::Cell<Option<u64>>,
    /// A jump move the cursor machine already took
    /// ([`OriginalUi::cursor_jump_moved`]).
    cursor_moved_now: std::cell::Cell<Option<Point>>,
    /// The client quest flags (S→C 0x29), as the waypoint
    /// tab gates read them.
    client_quest: [u8; 96],
    /// `levels` `LevelName` keys by level id (the waypoint rows' text,
    /// `waypoint_ui`).
    level_names: Vec<String>,
    /// The palette index of the monster health bar's red (`set_palette`).
    health_bar_color: Option<u8>,
    /// The skill icon under the mouse this frame, for the pop-up layer
    /// (`skill_hover`); written by the panel that draws the icon.
    skill_hover: std::cell::Cell<Option<skill_hover::SkillHover>>,
    /// The skill button under the mouse (`skill_hover::ButtonHover`).
    button_hover: std::cell::Cell<Option<skill_hover::ButtonHover>>,
    /// A skill pop-up took the one pop-up slot this frame, after the world
    /// hover label's step 1 write.
    popup_taken: std::cell::Cell<bool>,
    /// Ran (an `hst ` / `qf2 ` arrived in page 3): the
    /// animation starts at the cube's next draw.
    horadric_start: std::cell::Cell<bool>,
}

impl Shared {
    /// The clock of the UI's timers ([`super::ui_clock`]) at bridge frame
    /// `frames` (a draw's [`UiCtx::tick`], else the world's frame count):
    /// the live wall clock, else the frame count at 40 ms.
    pub(super) fn ui_ms(&self, frames: u64) -> u64 {
        super::ui_clock::ui_ms(self.menu_wall_ms.get(), frames)
    }

    /// [`Self::ui_ms`] as the 32-bit millisecond clock.
    pub(super) fn ui_ms32(&self, frames: u64) -> u32 {
        self.ui_ms(frames) as u32
    }

    /// The shop's UI flag (the NPC menu box's Esc delegation).
    pub(super) fn shop_flag_open(&self) -> bool {
        self.states.is_open(super::states::id::NPC_SHOP)
    }

    fn env(&self) -> PanelEnv {
        PanelEnv {
            screen: self.config.screen,
            open_mode: self.states.open_mode().get(),
            exp: self.facts.expansion_game && self.config.expansion_installed,
        }
    }

    /// The right panel's area: the `inv` rectangle of the local
    /// player's record; none without the table or a player.
    fn right_area(&self) -> Option<Rect> {
        let class = self.facts.class?;
        let r = inventory_record(class, &self.config.screen)?;
        Some(self.inv_areas.as_ref()?.get(r)?.rect())
    }

    fn gate_env(&self) -> GateEnv {
        GateEnv {
            expansion: self.facts.expansion_game && self.config.expansion_installed,
            player: self.facts.player,
            // Only ui 5 / 9 read these two; the controller never
            // requests them (`PENDING`).
            chat_blocked: false,
            input_hold: false,
            // D2RUST has no modal text screen and no NPC interaction yet.
            modal_text: false,
            npc_active: false,
            screen: self.config.screen,
            mouse: self.mouse,
        }
    }
}

type SharedRef = Rc<RefCell<Shared>>;

/// What the controller did besides the root's intents, for the frame's
/// caller (world view, audio) and tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UiOutcome {
    /// `SetUIState` effects, in call order.
    pub effects: Vec<UiEffect>,
    /// Sound requests for the audio side, in order.
    pub sounds: Vec<SoundRequest>,
    /// Parts of the bridge outputs' UI dispatch not run because their
    /// input or callee is not settled yet (`msg_ui::skip`), in order.
    pub skipped: Vec<&'static str>,
}

/// The play UI: flags, tables and panel state.
pub struct OriginalUi {
    shared: SharedRef,
    outcome: UiOutcome,
    /// The UI globals of the bridge outputs.
    msg: MsgUiState,
    /// The UI globals of the smaller outputs (`msg_ui::more`).
    more: MsgUiMore,
    /// The NPC text list.
    npc_text: Option<NpcTextList>,
    /// The case of 0x28's dialog branch this UI chose for the last `NpcDialog`,
    /// for the bridge (open question 10 decided as A).
    dialog_answer: Option<(Box<NpcDialog>, DialogCase)>,
    /// The hire list (`ui/hire_list.rs`).
    pub(super) hire: super::hire_list::SharedHire,
    /// The NPC shop (`shop_ui`).
    shop: shop_ui::SharedShop,
    /// The NPC menu (`npc_box`).
    pub(super) npcm: npc_box::SharedNpcMenu,
    /// The C→S messages of the close hooks not yet
    /// handed to the root ([`Self::flush_hooks`]).
    hook_intents: Vec<super::ClientIntent>,
    /// The key mode (1 in play) and its keep-key-up flag, moved by the
    /// panels' open / close hooks.
    key_mode: (u8, bool),
    /// The mod game's client hooks (`crate::mods`); empty in D2RUST's own
    /// game.
    mods: ClientMods,
}

#[derive(Debug, thiserror::Error)]
pub enum OriginalUiError {
    #[error(transparent)]
    Layout(#[from] LayoutError),
    #[error(transparent)]
    Root(#[from] UiError),
    #[error(transparent)]
    State(#[from] UiStateError),
    /// The waypoint record's load copy refused its magic (a fatal error in
    /// the game).
    #[error(transparent)]
    Waypoint(d2_sim::world::waypoints::WaypointError),
    /// 0x28's dialog branch found no NPC text list (fatal error 0x1060 in
    /// the game).
    #[error("0x28 NPC dialog without an NPC text list (fatal 0x1060)")]
    NoNpcText,
    /// 0x27's list build asserts at a count of 8 or more.
    #[error("0x27 NPC text list with invalid count {0}")]
    NpcTextCount(u8),
}

impl OriginalUi {
    /// Loads the tables; all flags 0, open mode 0. `inv_areas`: the
    /// `inventory.bin` `inv` rectangles by record, `None` when the table is not
    /// loaded (the right panels then take no pointer event).
    pub fn new(config: UiConfig, inv_areas: Option<Vec<InvArea>>) -> Result<Self, LayoutError> {
        let mut tables = PanelTables::load()?;
        tables.files.extend(hud::hud_files());
        tables.files.extend(esc_art::esc_files());
        tables.files.extend(quest_log_ui::quest_files());
        tables.files.extend(cube_ui::cube_files());
        tables.files.extend(super::box_border::border_files());
        tables.files.extend(msg_log_ui::msg_log_files());
        tables.files.extend(npc_talk::socket_files());
        tables.files.extend(cursor_files());
        tables.files.extend(skill_tree_ui::icon_files());
        // The 16:9 side bars' own frame, made from the panel art (last, so
        // no other file's id moves).
        tables
            .files
            .extend([super::panels::border::SIDE_FILE.to_string()]);
        // Mini panel start: the game entry set-up opens the mini panel (state 0x15) and
        // then the help button (0x22). Without a registry neither `Mini
        // Panel` nor `Help Menu` is set at the start; a check run replays
        // the host's registry ([`OriginalUi::set_registry`]).
        let mut states = UiStates::new()?;
        states.force(UI_MINI_PANEL, true);
        states.force(crate::ui::panels::control::buttons::UI_HELP_BUTTON, true);
        let base_inv_areas = inv_areas.clone();
        let inv_areas = inv_areas.map(|v| {
            v.into_iter()
                .map(|a| at_edges_area(&config.screen, a))
                .collect()
        });
        let shared = Shared {
            tables,
            states,
            config,
            inv_areas,
            base_inv_areas,
            base_layouts: None,
            base_equip: None,
            facts: Facts {
                class: None,
                player: None,
                expansion_game: false,
                local: None,
                player_room: false,
                has_hireling: false,
                has_belt: false,
            },
            // The cursor init sets the mouse to
            // (W / 2, H / 2) of the 640 × 480 front end, the position the
            // cursor jump of a panel open reads until the first move
            // (measured: a tip drawn at (120, 237) after the start at (320, 240)).
            mouse: Point::new(320, 240),
            outputs: Vec::new(),
            input_reset: false,
            clear_automap: false,
            fonts: None,
            resist_penalties: None,
            char_tables: Default::default(),
            hud: hud::HudState::default(),
            skill_tree: Default::default(),
            items: ItemsUi::default(),
            waypoint_map: None,
            waypoint_open: None,
            esc: esc_menu::EscState::default(),
            esc_kept: Vec::new(),
            quest: quest_log_ui::QuestInputs::default(),
            gold: gold_dialog::GoldState::default(),
            messages: Default::default(),
            bubbles: Default::default(),
            cube_opened: false,
            cube_anim: Default::default(),
            // Init with adj 0 (the only caller passes 0). The init runs before
            // the resolution is set, so W x H is the 640 x 480 default, not the
            // screen.
            cursor: RefCell::new(super::cursor::Cursor::init(0, 640, 480, 0)),
            cursor_seed: std::cell::Cell::new(None),
            client_seed: None,
            cursor_step_owed: std::cell::Cell::new(false),
            cursor_hold: std::cell::Cell::new(None),
            cursor_item_drawn: std::cell::Cell::new(false),
            host_now: std::cell::Cell::new(None),
            menu_wall_ms: std::cell::Cell::new(None),
            cursor_moved_now: std::cell::Cell::new(None),
            client_quest: [0; 96],
            level_names: Vec::new(),
            health_bar_color: None,
            skill_hover: Default::default(),
            button_hover: Default::default(),
            popup_taken: Default::default(),
            horadric_start: Default::default(),
        };
        Ok(Self {
            shared: Rc::new(RefCell::new(shared)),
            outcome: UiOutcome::default(),
            msg: MsgUiState::default(),
            more: MsgUiMore::default(),
            npc_text: None,
            dialog_answer: None,
            hire: super::hire_list::SharedHire::default(),
            shop: shop_ui::SharedShop::default(),
            npcm: Default::default(),
            hook_intents: Vec::new(),
            // A game starts in key mode 1.
            key_mode: (1, false),
            mods: ClientMods::default(),
        })
    }

    /// Adds the play UI's panels to `root`, bottom-most first.
    ///
    /// The add order is the draw order, and the root routes input to the
    /// last-added panel first: the equipment warning icons, the quest log,
    /// the inventory family (inventory, stash, store, cube), skill tree,
    /// character and waypoint panels, the border and control panel with
    /// the HUD overlays (and a mod game's HUD lines, `crate::mods`), the
    /// NPC boxes, gold dialog, message log, screen messages, overhead
    /// bubbles and NPC dialog, a mod game's added panels, the Esc menu, and
    /// last the top layer with the cursor. The Esc menu sits this high
    /// because its input routing depends on it. A mod game's replacement
    /// panels then take the place of the panels they replace.
    pub fn install(&self, root: &mut UiRoot) -> Result<(), UiError> {
        let sh = &self.shared;
        root.add(Box::new(hud::WarnUi { sh: sh.clone() }))?;
        root.add(Box::new(quest_log_ui::QuestLogUi {
            sh: sh.clone(),
            log: Default::default(),
            shown: Default::default(),
            close_down: Default::default(),
        }))?;
        root.add(Box::new(InventoryUi {
            sh: sh.clone(),
            shop: self.shop.clone(),
            panel: InventoryPanel::default(),
        }))?;
        root.add(Box::new(stash_ui::StashUi {
            sh: sh.clone(),
            input: Default::default(),
        }))?;
        root.add(Box::new(shop_ui::ShopUi {
            sh: sh.clone(),
            st: self.shop.clone(),
        }))?;
        root.add(Box::new(cube_ui::CubeUi {
            sh: sh.clone(),
            input: Default::default(),
        }))?;
        root.add(Box::new(SkillTreeUi {
            sh: sh.clone(),
            panel: SkillTreePanel::new(),
        }))?;
        root.add(Box::new(CharacterUi {
            sh: sh.clone(),
            panel: CharacterPanel::default(),
        }))?;
        root.add(Box::new(waypoint_ui::WaypointUi {
            sh: sh.clone(),
            panel: Default::default(),
            seq: 0,
        }))?;
        root.add(Box::new(BorderUi { sh: sh.clone() }))?;
        root.add(Box::new(hud::HudUi { sh: sh.clone() }))?;
        // Only with a mod game's HUD lines, so D2RUST's own root is
        // unchanged.
        if self.mods.has_hud_lines() {
            root.add(Box::new(super::mod_hud::ModHudUi {
                mods: self.mods.clone(),
            }))?;
            root.open(super::mod_hud::MOD_HUD_PANEL)?;
        }
        root.add(Box::new(super::hire_list::HireListUi {
            st: self.hire.clone(),
            border: sh
                .borrow()
                .tables
                .files
                .id(super::box_border::BOXPIECES_FILE),
        }))?;
        root.open(super::hire_list::HIRE_PANEL)?;
        root.add(Box::new(npc_box::NpcMenuUi {
            sh: sh.clone(),
            shop: self.shop.clone(),
            st: self.npcm.clone(),
            hire: self.hire.clone(),
        }))?;
        root.add(Box::new(npc_talk::SocketUi {
            sh: sh.clone(),
            st: self.npcm.clone(),
        }))?;
        root.add(Box::new(gold_dialog::GoldDialogUi { sh: sh.clone() }))?;
        root.add(Box::new(msg_log_ui::MsgLogUi::new(sh.clone())))?;
        root.add(Box::new(game_messages::MessagesUi { sh: sh.clone() }))?;
        root.open(game_messages::MESSAGES_PANEL)?;
        root.add(Box::new(overhead_ui::OverheadUi { sh: sh.clone() }))?;
        root.open(overhead_ui::OVERHEAD_PANEL)?;
        // The text pass after the overhead bubbles.
        root.add(Box::new(npc_talk::DialogPanelUi {
            sh: sh.clone(),
            st: self.npcm.clone(),
        }))?;
        root.open(npc_talk::DIALOG_PANEL)?;
        // A mod game's own panels: above the game's, below the Esc menu
        // and the cursor.
        let files = self.files();
        for (place, factory) in self.mods.panels() {
            if let PanelPlace::Add { open } = place {
                let panel = factory.make(&files);
                let id = panel.id();
                root.add(panel)?;
                if open {
                    root.open(id)?;
                }
            }
        }
        root.add(Box::new(esc_menu::EscMenuUi {
            sh: sh.clone(),
            pent: Default::default(),
        }))?;
        // The top layer and the cursor draw over every panel.
        root.add(Box::new(TopUi {
            sh: sh.clone(),
            shop: self.shop.clone(),
        }))?;
        // A mod game's replacements take their panel's place.
        for (place, factory) in self.mods.panels() {
            if place == PanelPlace::Replace {
                root.replace(factory.make(&files))?;
            }
        }
        // Not a UI state: open for good.
        root.open(BORDER_PANEL)?;
        root.open(hud::HUD_PANEL)?;
        root.open(hud::WARN_PANEL)?;
        root.open(TOP_PANEL)?;
        root.open(gold_dialog::GOLD_PANEL)?;
        root.sync_states(&sh.borrow().states);
        let sc = sh.borrow().config.screen;
        {
            let shr = sh.borrow();
            let mut h = self.hire.borrow_mut();
            h.screen = (sc.w, sc.h);
            h.fonts = shr.fonts.clone();
        }
        self.npcm.borrow_mut().screen = (sc.w, sc.h);
        Ok(())
    }

    /// The panel file registry ([`super::ImageRef::file`] ids).
    pub fn files(&self) -> UiFiles {
        self.shared.borrow().tables.files.clone()
    }

    /// The glyph widths of the fonts the panels measure (the character panel's
    /// values and name line). Without them those texts are not drawn. The draws
    /// name fonts ([`CHARACTER_FONTS`]) whose `.tbl` and DC6 the frame's assets
    /// must then hold ([`crate::world_view::ui_bind::TextAssetLoader`]). Links
    /// the cursor step to the client seed the weather and sound draws share
    /// (one random stream).
    pub fn set_client_seed(
        &mut self,
        seed: std::sync::Arc<std::sync::Mutex<crate::audio::driver::ClientSeed>>,
    ) {
        self.shared.borrow_mut().client_seed = Some(seed);
    }

    /// The UI's pointer in frame space: the last cursor event, (320, 240)
    /// of the UI box at the start.
    pub fn pointer(&self) -> Point {
        let p = self.shared.borrow().mouse;
        Point::new(p.x + FrameSize::play().ui_left(), p.y)
    }

    /// The glyph widths bound by [`OriginalUi::set_fonts`], if any.
    pub fn font_measure(&self) -> Option<FontMeasure> {
        self.shared.borrow().fonts.clone()
    }

    pub fn set_fonts(&mut self, fonts: FontMeasure) {
        self.hire.borrow_mut().fonts = Some(fonts.clone());
        self.shared.borrow_mut().fonts = Some(fonts);
    }

    /// `difficultylevels` `ResistPenalty` per difficulty, in row order (the
    /// expansion penalty of the resistance values).
    pub fn set_resist_penalties(&mut self, penalties: Vec<i32>) {
        self.shared.borrow_mut().resist_penalties = Some(penalties);
    }

    /// The character panel's class keys, state flags and skilldesc rows.
    pub fn set_char_tables(&mut self, tables: super::char_feed::CharTables) {
        self.shared.borrow_mut().char_tables = tables;
    }

    /// The HUD's skill icon and experience tables ([`hud::HudTables`]).
    pub fn set_hud_tables(&mut self, tables: hud::HudTables) {
        self.shared.borrow_mut().hud.tables = tables;
    }

    /// The skill tree's class skill rows (`skill_tree_ui`); without them
    /// the tree shows no icons.
    pub fn set_skill_tree_tables(&mut self, tables: skill_tree_ui::SkillTreeTables) {
        self.shared.borrow_mut().skill_tree = tables;
    }

    /// The walk's run toggle for the run button; returns the
    /// toggles the run button asked for since the last call.
    pub fn sync_run(&mut self, running: bool) -> u32 {
        let mut sh = self.shared.borrow_mut();
        sh.hud.running = running;
        std::mem::take(&mut sh.hud.run_toggles)
    }

    /// Item-table art rows (`inv_items`): the inventory panel draws the
    /// local player's items and the cursor item with them; empty draws
    /// nothing. Registers their graphics in [`Self::files`], so call it
    /// before handing the files to the art loader.
    pub fn set_item_art(&mut self, art: ItemArtRows) {
        let mut sh = self.shared.borrow_mut();
        sh.items.art = art;
        let Shared { items, tables, .. } = &mut *sh;
        items.register_files(&mut tables.files);
    }

    /// `inventory.bin` layouts by record (`inv_items::inv_layout` of each
    /// row); without them the grid is the built-in measured record.
    pub fn set_inv_layouts(&mut self, layouts: Vec<InvLayout>) {
        let mut sh = self.shared.borrow_mut();
        let s = sh.config.screen;
        sh.base_layouts = Some(layouts.clone());
        sh.items.layouts = Some(
            layouts
                .into_iter()
                .map(|l| at_edges_layout(&s, l))
                .collect(),
        );
    }

    /// The equipment rectangles by `inventory.bin` record: the empty
    /// equipment-slot pictures (a hand's two-handed test reads the inventory
    /// tables; [`Self::set_inv_tables`]). Call before handing the files to the
    /// art loader (the pictures' files are registered here).
    pub fn set_equip_rects(&mut self, rects: Vec<super::panels::inventory::EquipRects>) {
        let mut sh = self.shared.borrow_mut();
        if !rects.is_empty() {
            for f in super::panels::inventory::background_files() {
                sh.tables.files.add(f);
            }
        }
        let s = sh.config.screen;
        sh.base_equip = Some(rects.clone());
        sh.items.equip_rects = Some(rects.into_iter().map(|r| at_edges_equip(&s, r)).collect());
    }

    /// Refresh retained panel geometry without reinstalling panels or changing flags.
    pub fn resize_screen(&mut self, screen: Screen) {
        let mut sh = self.shared.borrow_mut();
        if sh.config.screen == screen {
            return;
        }
        sh.inv_areas = sh.base_inv_areas.as_ref().map(|v| {
            v.iter()
                .copied()
                .map(|a| at_edges_area(&screen, a))
                .collect()
        });
        sh.items.layouts = sh.base_layouts.as_ref().map(|v| {
            v.iter()
                .copied()
                .map(|l| at_edges_layout(&screen, l))
                .collect()
        });
        sh.items.equip_rects = sh.base_equip.as_ref().map(|v| {
            v.iter()
                .copied()
                .map(|r| at_edges_equip(&screen, r))
                .collect()
        });
        let pointer = screen.pointer_from(sh.config.screen, sh.mouse);
        sh.mouse = pointer;
        {
            let mut cursor = sh.cursor.borrow_mut();
            cursor.mx = pointer.x;
            cursor.my = pointer.y;
            cursor.host_at = None;
        }
        sh.cursor_hold.set(None);
        sh.cursor_moved_now.set(None);
        sh.config.screen = screen;
        sh.esc.menu.screen = screen;
        sh.input_reset = true;
        let fonts = sh.fonts.clone();
        if let Some(confirm) = self.shop.borrow_mut().confirm.as_mut() {
            confirm.pressed = None;
            npc_box::reflow_box(
                &mut confirm.bx,
                (screen.w, screen.h),
                (screen.w / 2, screen.h / 3),
                &game_messages::Measure(fonts.as_ref()),
            );
        }
        drop(sh);
        self.outcome
            .effects
            .retain(|e| !matches!(e, UiEffect::CursorX(_)));

        self.hire.borrow_mut().screen = (screen.w, screen.h);
        self.npcm.borrow_mut().resize_screen(screen, fonts.as_ref());
    }

    /// The `belts.bin` records and the belts' types (`hud_belt`).
    pub fn set_belt_parts(&mut self, parts: hud_belt::BeltParts) {
        self.shared.borrow_mut().hud.belt.parts = parts;
    }

    /// The act palette the UI's tint colours
    /// are matched in, set by the host each frame.
    pub fn set_palette(&mut self, palette: &d2_formats::palette::Palette) {
        let p: Vec<[u8; 3]> = palette.colors.iter().map(|c| [c.r, c.g, c.b]).collect();
        let mut sh = self.shared.borrow_mut();
        sh.items.tint_colors = super::inv_grid::tint_indices(&p);
        sh.health_bar_color = super::inv_grid::nearest_index(&p, (0x50, 0, 0));
        sh.hud.show_items_blue = super::inv_grid::nearest_index(&p, (0, 0x40, 0x80));
        // The stamina bar's colours.
        use super::panels::control::globes::StaminaColor;
        let n = |c: StaminaColor| super::inv_grid::nearest_index(&p, c.rgb());
        sh.hud.stamina_colors = match (
            n(StaminaColor::Red),
            n(StaminaColor::Gold),
            n(StaminaColor::Blue),
        ) {
            (Some(r), Some(g), Some(b)) => Some([r, g, b]),
            _ => None,
        };
    }

    /// The `levels` `LevelName` string key of level `id`
    /// ([`Self::set_level_names`]); `None` when unset or empty.
    pub fn level_name_key(&self, id: u16) -> Option<String> {
        let sh = self.shared.borrow();
        sh.level_names
            .get(usize::from(id))
            .filter(|k| !k.is_empty())
            .cloned()
    }

    /// The open panels as the monster label reads them: what the hovered
    /// monster's label may use (`world_view::monster_label`).
    pub fn label_free_screen(&self) -> crate::world_view::monster_label::FreeScreen {
        let sh = self.shared.borrow();
        let any = |ids: &[u8]| ids.iter().any(|&i| sh.states.is_open(i));
        crate::world_view::monster_label::FreeScreen {
            blocked: any(&[0x17, 0x19, 0x1A, 0x0C, 0x1C, 0x1B]),
            left: any(&[0x1D, 0x16, 2, 0x0F, 0x24, 0x25, 0x14]),
            right: any(&[1, 4]),
        }
    }

    /// The object mouse-over label is not drawn while a store is open
    /// (a store transaction is active) or the dialog panel is up.
    pub fn object_label_blocked(&self) -> bool {
        self.shop.borrow().open.is_some() || self.npcm.borrow().talk.dialog.panel.is_some()
    }

    /// A skill pop-up of the last UI frame
    /// overwrote the pop-up slot, so the world's hover label (the NPC
    /// name, monster bar or object label, written at step 1) is not drawn.
    pub fn popup_taken(&self) -> bool {
        self.shared.borrow().popup_taken.get()
    }

    /// The palette index of the monster health bar's red
    /// (palette colour 0x50 at full light), once the host has set the palette.
    pub fn health_bar_color(&self) -> Option<u8> {
        self.shared.borrow().health_bar_color
    }

    /// The frame's local-player position and shake (the world view's
    /// one camera), set before each UI frame.
    pub fn set_frame_anchor(&mut self, anchor: Option<crate::rules::camera::FrameAnchor>) {
        self.shared.borrow_mut().bubbles.anchor = anchor;
    }

    /// The belt key labels follow the play bindings (`hud_belt`). The run and
    /// mini-panel tips' key names too (`hud::key_names`).
    pub fn set_belt_keys(&mut self, b: &crate::controls::Bindings) {
        let mut sh = self.shared.borrow_mut();
        sh.hud.belt.set_keys(b);
        sh.hud.bindings = Some(b.clone());
    }

    /// The item tool tips' tables and strings (`item_tip`); the tips run
    /// the mod game's item hooks ([`Self::set_mods`]).
    pub fn set_item_tips(&mut self, mut tips: item_tip::ItemTips) {
        tips.mods = self.mods.clone();
        self.shared.borrow_mut().items.tips = Some(tips);
    }

    /// The mod game's client hooks (`crate::mods`). Set before
    /// [`Self::install`]: the HUD lines and mod panels are added there.
    pub fn set_mods(&mut self, mods: ClientMods) {
        let mut sh = self.shared.borrow_mut();
        if let Some(tips) = sh.items.tips.as_mut() {
            tips.mods = mods.clone();
        }
        // The mod panels' art goes last, so no file id of the game's own
        // panels moves.
        for (_, factory) in mods.panels() {
            sh.tables.files.extend(factory.files());
        }
        drop(sh);
        self.mods = mods;
    }

    /// The inventory tables of the equip-box click (`inv_items` `equip`).
    pub fn set_inv_tables(&mut self, t: std::sync::Arc<d2_sim::items::inventory::InvTables>) {
        self.shared.borrow_mut().items.inv_tables = Some(t);
    }

    /// Shift is held (set by the host each frame, `inv_items`).
    pub fn set_shift(&mut self, shift: bool) {
        self.shared.borrow_mut().items.shift = shift;
    }

    /// Whether Shift was held at the last [`Self::set_shift`].
    pub fn shift_held(&self) -> bool {
        self.shared.borrow().items.shift
    }

    /// Ctrl is held (set by the host each frame, `inv_items`).
    pub fn set_ctrl(&mut self, ctrl: bool) {
        self.shared.borrow_mut().items.ctrl = ctrl;
    }

    /// The centre of the backpack cell the player's page-0 item `guid`
    /// starts at (tests).
    pub fn inv_item_point(&self, world: &ClientWorld, guid: u32) -> Option<Point> {
        let sh = self.shared.borrow();
        let class = Facts::of(world).class;
        let l = sh.items.layout(class, &sh.config.screen)?;
        let it = crate::bridge::items::local_items(world)
            .into_iter()
            .find(|i| {
                i.key.guid == guid && i.mode == crate::bridge::items::mode::STORED && i.page == 0
            })?;
        let (x, y, w, h) = l.grid.cell(i32::from(it.x), i32::from(it.y));
        Some(Point::new(x + w / 2, y + h / 2))
    }

    /// The item tool tip the inventory draws now at the last mouse point,
    /// as text lines (the same lines as the tip pass; tests).
    pub fn inv_hover_tip(&self, world: &ClientWorld) -> Vec<String> {
        let sh = self.shared.borrow();
        let class = Facts::of(world).class;
        sh.items
            .hover_lines(world, &sh.tables.files, &sh.config.screen, class, sh.mouse)
            .iter()
            .map(|l| String::from_utf16_lossy(&l.text))
            .collect()
    }

    /// The identify cursor is on: a scroll or tome of identify waits for
    /// its target (cursor state 6, `inv_items_tip`; tests).
    pub fn identify_pending(&self) -> bool {
        self.shared.borrow().items.identify_pending()
    }

    /// Measured item graphic frame sizes by `invfile` (lower case).
    pub fn set_item_frame_sizes(&mut self, sizes: BTreeMap<String, (u32, u32)>) {
        self.shared.borrow_mut().items.frame_sizes = sizes;
    }

    /// The flags.
    pub fn is_open(&self, ui: u8) -> bool {
        self.shared.borrow().states.is_open(ui)
    }

    /// The screen open mode: the camera's.
    pub fn open_mode(&self) -> OpenMode {
        self.shared.borrow().states.open_mode()
    }

    /// The mouse move a cursor jump causes: the
    /// game handles the `WM_MOUSEMOVE` of its `SetCursorPos` in the same
    /// message pump, before the pass's frame, so the cursor machine takes
    /// it now; the queued [`UiEvent::CursorMoved`] that brings it
    /// to the other handlers skips the machine. `host_timed`: a check run
    /// whose schedule holds the cursor's idle start; the host delivers
    /// that message at its scheduled time (a jump in a client update
    /// has it handled at the next pump, after this frame's cursor step:
    /// a waypoint panel click), so the jump only places the pointer and
    /// the scheduled host message runs the handler. `jump`: the jump's own
    /// point, which the jump writes to the mouse variables before its
    /// `SetCursorPos`, so the host-timed frames up
    /// to that message draw it (a charged bolt cast: (790, 262)
    /// at tick 137, (790, 259) from 138).
    pub fn cursor_jump_moved(
        &mut self,
        jump: Point,
        at: Point,
        world: &ClientWorld,
        host_timed: bool,
    ) {
        // Both points are in frame space ([`Self::take_cursor_warp`]); the
        // cursor keeps UI-box space.
        let ox = FrameSize::play().ui_left();
        let (jump, at) = (Point::new(jump.x - ox, jump.y), Point::new(at.x - ox, at.y));
        if host_timed {
            self.shared
                .borrow()
                .cursor
                .borrow_mut()
                .jump_to(jump.x, jump.y, (at.x, at.y));
        } else {
            self.cursor_event(UiEvent::CursorMoved(at), world);
        }
        let sh = self.shared.borrow();
        sh.cursor_moved_now.set(Some(at));
    }

    /// Reads the model facts of the next event (call before routing it).
    pub fn before_event(&mut self, e: UiEvent, world: &ClientWorld) {
        self.refresh_facts(world);
        let taken = self.shared.borrow().cursor_moved_now.take();
        if !matches!((e, taken), (UiEvent::CursorMoved(p), Some(q)) if p == q) {
            self.cursor_event(e, world);
        }
        if let Some(p) = e.at() {
            self.shared.borrow_mut().mouse = p;
            // D2RUST's own plumbing for the grid hover: a
            // move or a press tracks the hover; a release does not, so a
            // use press's cleared hover holds until the mouse moves
            // (measured: a cube panel scene).
            if matches!(e, UiEvent::CursorMoved(_) | UiEvent::Press { .. }) {
                self.track_grid_hover(world, p);
            }
        }
    }

    /// The hover handler of every open
    /// item grid under `p`: the inventory (page 0), stash (page 4) and
    /// cube (page 3) grids, so the grid click reads the kept cursor cell
    /// and the backpack's hover tip and tint find their item. The backpack
    /// is drawn whenever a panel of the inventory family is open (the shop,
    /// stash and cube show it beside their own grid without setting the
    /// inventory flag), so it tracks the hover in all of those cases too.
    fn track_grid_hover(&self, world: &ClientWorld, p: Point) {
        use super::states::id;
        let sh = self.shared.borrow();
        let s = sh.config.screen;
        let files = &sh.tables.files;
        if sh.states.is_open(id::STASH) {
            let g = sh.items.stash_grid(sh.env().exp, &s);
            sh.items
                .track_hover(world, files, &g, super::panels::stash_items::STASH_PAGE, p);
        }
        // The shop's move handlers run the
        // same handler over the store grid.
        shop_ui::track_store_hover(&sh, world, &self.shop.borrow(), p);
        if sh.states.is_open(id::CUBE) {
            let g = sh.items.cube_grid(&s);
            sh.items
                .track_hover(world, files, &g, super::panels::cube_items::CUBE_PAGE, p);
        }
        let backpack_shown = super::root::INVENTORY_FAMILY
            .iter()
            .any(|&u| sh.states.is_open(u));
        if backpack_shown {
            if let Some(l) = sh.items.layout(Facts::of(world).class, &s) {
                sh.items.track_hover(world, files, &l.grid, 0, p);
            }
        }
    }

    /// Reads the model facts the gate uses.
    fn refresh_facts(&mut self, world: &ClientWorld) {
        self.shared.borrow_mut().facts = Facts::of(world);
    }

    /// Applies what the routed event asked for (module doc): panel
    /// outputs in order, then a hotkey action no panel took; the root
    /// mirrors the flags after.
    pub fn after_event(
        &mut self,
        root: &mut UiRoot,
        e: UiEvent,
        routed: Routed,
    ) -> Result<(), OriginalUiError> {
        let outputs = std::mem::take(&mut self.shared.borrow_mut().outputs);
        for o in outputs {
            match o {
                PanelOutput::Intent(i) => root.queue_intent(i),
                // A refused call (returns 0) changes nothing.
                // The mini panel's menu button opens the game menu through.
                PanelOutput::SetUi { ui, mode: 0, .. }
                    if ui == super::states::id::ESC_MENU && !self.is_open(ui) =>
                {
                    self.open_game_menu()?;
                }
                PanelOutput::SetUi { ui, mode, jump } => {
                    let was_menu = ui == super::states::id::ESC_MENU && self.is_open(ui);
                    // The panel's outputs already carry its own close
                    // hook's message (`stash_input`, `waypoint`).
                    self.set_ui_from(u32::from(ui), u32::from(mode), jump, Some(ui))?;
                    // Return to Game closes the menu through.
                    if was_menu && !self.is_open(ui) {
                        self.restore_game_menu_states()?;
                    }
                }
                PanelOutput::Sound(id) => self.outcome.sounds.push(SoundRequest::Ui(id)),
                PanelOutput::DialogSpeechEnd => {
                    self.outcome.sounds.push(SoundRequest::NpcDialogClose);
                }
                PanelOutput::PlayerEvent(event) => {
                    let local = self.shared.borrow().facts.local;
                    if let Some(unit) = local {
                        self.outcome
                            .sounds
                            .push(SoundRequest::PlayerEvent { unit, event });
                    }
                }
            }
        }
        if let (Routed::Unhandled, UiEvent::Action(a)) = (routed, e) {
            if a == ActionId(Action::GameMenu.index() as u16) {
                self.game_menu_key()?;
            } else if a == ActionId(Action::ClearScreen.index() as u16) {
                // Command 38: the
                // close-all (0, 1); when it closed nothing, the automap
                // re-centre and the close-all with the automap (the host).
                if !self.close_all(true)? {
                    self.shared.borrow_mut().clear_automap = true;
                }
            } else if a == ActionId(Action::ToggleHireling.index() as u16) {
                // Command 54: an expansion game, a hireling
                // and the expansion installed.
                let ok = {
                    let sh = self.shared.borrow();
                    sh.facts.expansion_game
                        && sh.facts.has_hireling
                        && sh.config.expansion_installed
                };
                if ok {
                    self.set_ui(u32::from(super::states::id::MERC_INV), 2, true)?;
                }
            } else if a == ActionId(Action::ToggleBelt.index() as u16) {
                // Command 22: a worn belt, then
                // SetUIState(0x1F, toggle, 0).
                if self.shared.borrow().facts.has_belt {
                    self.set_ui(0x1F, 2, false)?;
                }
            } else if a == ActionId(Action::ToggleSkillMenuRight.index() as u16) {
                // Command 13:
                // SetUIState(3, toggle, 0), then the right skill button's
                // pick, as its release.
                self.set_ui(3, 2, false)?;
                self.shared.borrow_mut().hud.select_left = false;
            } else if a == ActionId(Action::ToggleHelp.index() as u16) {
                // Command 6: SetUIState(0x21, toggle, 0); `Help Menu`:= 1 when
                // it was absent or 0.
                self.set_ui(u32::from(help_screen::STATE), 2, false)?;
                self.shared.borrow_mut().hud.help.setting = 1;
            } else if let Some(ui) = hotkey_state(a) {
                // The Character, Inventory, Party, Skill Tree and
                // Hireling keys pass jump 1, and the Quest Log key too
                // (the quest log toggle hands its argument to SetUIState on the
                // open and on the close); every
                // other hot key 0; mode 2 toggle.
                let jump = matches!(ui, 1 | 2 | 4 | 0x0F | 0x16 | 0x24);
                self.set_ui(u32::from(ui), 2, jump)?;
                // The quest log asks the server for the quest data when
                // it opens (`quest_log_ui`; D2RUST's own, unverified).
                if ui == quest_log_ui::UI_QUEST_SCREEN && self.is_open(ui) {
                    root.queue_intent(quest_log_ui::request_quest_data());
                }
            }
        }
        self.flush_hooks(root);
        root.sync_states(&self.shared.borrow().states);
        Ok(())
    }

    /// Hands the close hooks' messages to the root, in call order.
    pub fn flush_hooks(&mut self, root: &mut UiRoot) {
        for i in self.hook_intents.drain(..) {
            root.queue_intent(i);
        }
    }

    /// The close hook, the parts that send: stash (ui 0x19, r7: only in
    /// inventory mode 0x0C / 0x0D, mode:= 0, C→S 0x4F 0x12), cube (ui 0x1A,
    /// the close: mode:= 0, the latched 0x4F 0x17 sent) and
    /// waypoint (ui 0x14: the latched 0x49 level 0, with a player
    /// and its room). `send` false: the caller's outputs carry the message
    /// already (a panel's own close path); the mode is still reset.
    ///
    /// D2RUST's own reading: the latches (the cube close
    /// sent) live in the panels; every panel path that sets one closes its
    /// state in the same outputs (and its hook runs with `send` false), so on
    /// any other close the latch is clear and the hook sends. "Its room" is
    /// read as the local player having a position.
    fn close_hook(&mut self, ui: u8, send: bool) {
        use super::states::id;
        let msg = match ui {
            id::STASH => {
                if !matches!(
                    self.msg.inventory_mode,
                    msg_ui::MODE_STASH | msg_ui::MODE_STASH_2
                ) {
                    return;
                }
                self.msg.inventory_mode = 0;
                super::ClientIntent::from_message(&d2_proto::client::ClickButton {
                    button: 0x12,
                    p1: 0,
                    p2: 0,
                })
            }
            id::CUBE => {
                self.msg.inventory_mode = 0;
                super::ClientIntent::from_message(&d2_proto::client::ClickButton {
                    button: 0x17,
                    p1: 0,
                    p2: 0,
                })
            }
            id::WAYPOINT => {
                let sh = self.shared.borrow();
                let Some(open) = sh.waypoint_open.filter(|_| sh.facts.player_room) else {
                    return;
                };
                super::ClientIntent::from_message(&d2_proto::client::TakeOrCloseWp {
                    wp: open.guid,
                    level: 0,
                })
            }
            _ => return,
        };
        if send {
            self.hook_intents.push(msg);
        }
    }

    /// Esc (command 56): with ui 9 open
    /// the menu closes and the remembered states reopen; else
    /// the close-all runs and the
    /// menu opens only when it closed nothing. The command's
    /// no-op while an NPC interaction or a modal text screen is active is
    /// not modelled (the NPC menu is not ui 8 in play).
    fn game_menu_key(&mut self) -> Result<(), OriginalUiError> {
        use super::states::id;
        if self.is_open(id::ESC_MENU) {
            self.set_ui(u32::from(id::ESC_MENU), 1, false)?;
            self.restore_game_menu_states()?;
            return Ok(());
        }
        if !self.close_all(true)? {
            self.open_game_menu()?;
        }
        Ok(())
    }

    /// The close-all of the
    /// Esc-closable states; whether it closed one.
    fn close_all(&mut self, jump: bool) -> Result<bool, UiStateError> {
        let mut closed = false;
        for ui in ESC_CLOSABLE {
            if self.is_open(ui) {
                self.set_ui(u32::from(ui), 1, jump)?;
                closed = true;
            }
        }
        Ok(closed)
    }

    /// The Show Items labels of the ground items: `None` unless Show Items
    /// (ui 0x0D) is on and the item tables and fonts are bound. `cam` is
    /// the frame's camera, `pointer` the UI pointer, `hovered` the hover
    /// unit of the pass before (an item's GUID).
    pub fn ground_item_labels(
        &self,
        world: &ClientWorld,
        cam: &crate::rules::camera::Camera,
        pointer: (i32, i32),
        hovered: Option<u32>,
    ) -> Option<super::show_items::Layout> {
        if !self.is_open(0x0D) {
            return None;
        }
        let sh = self.shared.borrow();
        let tips = sh.items.tips.as_ref()?;
        let fonts = sh.fonts.as_ref()?;
        let me = super::item_tip_world::WorldUnit::local(world);
        // The labels come from the rooms next to the player's room, each
        // room's units newest first, item units only: that order decides
        // which label wins a collision.
        let mut ground: BTreeMap<u32, crate::bridge::items::ItemView> =
            crate::bridge::items::ground_items(world)
                .into_iter()
                .map(|v| (v.key.guid, v))
                .collect();
        let rooms = match (world.drlg.as_ref(), world.local_room()) {
            (Some(cd), Some(own)) => cd.view_rooms(own.room),
            _ => Vec::new(),
        };
        let mut feed = Vec::new();
        let walk = rooms.iter().flat_map(|&r| world.room_units.list(r).iter());
        for k in walk.filter(|k| k.unit_type == 4) {
            let Some(v) = ground.remove(&k.guid) else {
                continue;
            };
            let Some((text, color)) = ground_label(tips, world, me.as_ref(), &v) else {
                continue;
            };
            feed.push(super::show_items::LabelIn {
                guid: v.key.guid,
                feet: crate::bridge::hover::unit_feet(cam, v.key.unit_type, (v.x, v.y)),
                text,
                color,
            });
        }
        let measure = |t: &[u16]| fonts.label_metrics(super::show_items::LABEL_FONT, t);
        Some(super::show_items::layout(
            &feed,
            &measure,
            {
                // The labels sit over ground items: frame space, the
                // whole frame (800 × 600 at 4:3, as the labels always used).
                let f = crate::scene::Rect::world_clip();
                (f.width as i32, f.height as i32)
            },
            pointer,
            hovered,
            sh.hud.show_items_blue.unwrap_or(0),
        ))
    }

    /// The label text of every ground item the client knows, by GUID:
    /// the text a Show Items label draws for it (the item's name, a gold
    /// pile's amount), whether or not Show Items is on. Empty until the
    /// item tables are bound. A read for test tools (`repro` checks); the
    /// labels themselves are drawn by [`Self::ground_item_labels`].
    pub fn ground_label_texts(&self, world: &ClientWorld) -> BTreeMap<u32, String> {
        let sh = self.shared.borrow();
        let Some(tips) = sh.items.tips.as_ref() else {
            return BTreeMap::new();
        };
        let me = super::item_tip_world::WorldUnit::local(world);
        crate::bridge::items::ground_items(world)
            .iter()
            .filter_map(|v| {
                let (text, _) = ground_label(tips, world, me.as_ref(), v)?;
                Some((v.key.guid, String::from_utf16_lossy(&text)))
            })
            .collect()
    }

    /// Show Items (command 37): the down handler sets
    /// ui 0x0D on, the up handler off; `held` is the input's state this
    /// pass, a change runs the handler.
    pub fn set_show_items(&mut self, held: bool) -> Result<(), UiStateError> {
        const UI_SHOW_ITEMS: u8 = 0x0D;
        if held != self.is_open(UI_SHOW_ITEMS) {
            self.set_ui(u32::from(UI_SHOW_ITEMS), if held { 0 } else { 1 }, false)?;
        }
        Ok(())
    }

    /// Whether Clear Screen closed nothing since the last call (its automap
    /// part, is the host's).
    pub fn take_clear_automap(&mut self) -> bool {
        std::mem::take(&mut self.shared.borrow_mut().clear_automap)
    }

    /// Every
    /// ui but 0 and 9 closes, the open ones of [`GAME_MENU_KEEP`] are
    /// remembered, then ui 9 opens.
    fn open_game_menu(&mut self) -> Result<(), OriginalUiError> {
        use super::states::id;
        let mut kept = Vec::new();
        for ui in 0..=37u8 {
            if ui == id::GAME || ui == id::ESC_MENU || !self.is_open(ui) {
                continue;
            }
            if GAME_MENU_KEEP.contains(&ui) {
                kept.push(ui);
            }
            self.set_ui(u32::from(ui), 1, false)?;
        }
        self.shared.borrow_mut().esc_kept = kept;
        self.set_ui(u32::from(id::ESC_MENU), 0, false)?;
        Ok(())
    }

    /// After ui 9 closed:
    /// the states remembered at the open reopen with jump 0.
    fn restore_game_menu_states(&mut self) -> Result<(), OriginalUiError> {
        let kept = std::mem::take(&mut self.shared.borrow_mut().esc_kept);
        for ui in kept {
            self.set_ui(u32::from(ui), 0, false)?;
        }
        Ok(())
    }

    /// Whether "Save and Exit Game" was chosen since the last call.
    pub fn take_exit_request(&mut self) -> bool {
        std::mem::take(&mut self.shared.borrow_mut().esc.exit_requested)
    }

    /// Whether Configure Controls was chosen since the last call.
    pub fn take_controls_request(&mut self) -> bool {
        std::mem::take(&mut self.shared.borrow_mut().esc.controls_requested)
    }

    /// Open the Configure Controls screen over the menu when it was
    /// chosen (`controls_host`); `path` is the `controls.toml` to load and
    /// save. Returns whether it opened.
    pub fn service_controls(&mut self, expansion: bool, path: Option<std::path::PathBuf>) -> bool {
        if !self.take_controls_request() {
            return false;
        }
        let mut sh = self.shared.borrow_mut();
        let mut c = controls_host::ControlsHost::open(expansion, path);
        c.set_wall_ms(sh.menu_wall_ms.get());
        sh.esc.controls = Some(c);
        true
    }

    /// Whether the Configure Controls screen is open.
    pub fn controls_open(&self) -> bool {
        self.shared.borrow().esc.controls.is_some()
    }

    /// Whether the expansion is installed (the Options and Configure Controls
    /// tables).
    pub fn expansion_installed(&self) -> bool {
        self.shared.borrow().config.expansion_installed
    }

    /// The open Controls screen's state (hosts and tests read it).
    pub fn controls_screen(
        &self,
    ) -> Option<crate::ui::front_end::screens::controls::ConfigureControls> {
        let sh = self.shared.borrow();
        sh.esc.controls.as_ref().map(|c| c.model().clone())
    }

    /// A raw key (Windows virtual key) for the open Controls screen;
    /// false when it is not open (the key is for the game).
    /// The key is stamped with the screen's UI clock, the one its draw
    /// times the error line by ([`controls_host::ControlsHost::now_ms`]).
    pub fn controls_key(&mut self, vk: u16) -> bool {
        let mut sh = self.shared.borrow_mut();
        let Some(c) = sh.esc.controls.as_mut() else {
            return false;
        };
        let now_ms = c.now_ms();
        if let Some(f) = c.key(vk, now_ms) {
            sh.esc.close_controls(f);
        }
        true
    }

    /// The play bindings accepted on the Controls screen, once.
    pub fn take_accepted_bindings(&mut self) -> Option<crate::controls::Bindings> {
        self.shared.borrow_mut().esc.accepted.take()
    }

    /// The host's registry values (`play --registry`), before the
    /// first frame: `Mini Panel` ≠ 0 keeps the mini panel closed at entry,
    /// `Help Menu` fills the help button's cache, `PopupHireling` the hire
    /// pop-up flag. Absent values keep the D2RUST start (no registry: all
    /// absent).
    pub fn set_registry(&mut self, r: crate::app::registry::UiRegistry) {
        if r.mini_panel.is_some_and(|v| v != 0) {
            self.shared.borrow_mut().states.force(UI_MINI_PANEL, false);
        }
        self.shared.borrow_mut().hud.help.setting = r.help_menu.unwrap_or(0);
        self.more_mut().popup_hireling = r.popup_hireling.unwrap_or(0);
    }

    /// The settings the Esc menu's Options page shows (`app::config`).
    /// The control panel's text toggles are read from them at start.
    pub fn set_settings(&mut self, s: crate::app::config::Settings) {
        let mut sh = self.shared.borrow_mut();
        sh.esc.menu.set_settings(s);
        sh.hud.input.show_hp = s.show_hp_text != 0;
        sh.hud.input.show_mp = s.show_mp_text != 0;
    }

    /// Whether the sound device is up: the Options pages' sound rows are
    /// enabled only then.
    pub fn set_sound_device(&mut self, up: bool) {
        self.shared.borrow_mut().esc.menu.sound_device = up;
    }

    /// The monitor's physical size, when known: the Window Size row offers
    /// only what fits on it.
    pub fn set_monitor(&mut self, monitor: Option<(u32, u32)>) {
        self.shared.borrow_mut().esc.menu.monitor = monitor;
    }

    /// The refresh rate of the window's monitor, for the Frame Limit row.
    pub fn set_refresh_hz(&mut self, hz: Option<u32>) {
        self.shared.borrow_mut().esc.menu.refresh_hz = hz;
    }

    /// The window as the platform reports it now, for the Video page's
    /// info lines and Window Size row.
    pub fn set_window_facts(&mut self, window: options_menu::WindowFacts) {
        self.shared.borrow_mut().esc.menu.window = window;
    }

    /// The settings after a change on the Options page, once per change.
    pub fn take_settings_change(&mut self) -> Option<crate::app::config::Settings> {
        let mut sh = self.shared.borrow_mut();
        sh.esc.menu.take_changed()
    }

    /// `SetUIState(ui, mode, jump)` with the model's gate facts; effects
    /// are kept for [`Self::take_outcome`].
    /// Every state it closes runs its close hook ([`Self::close_hook`]);
    /// the hooks' messages leave at the next [`Self::flush_hooks`].
    pub fn set_ui(&mut self, ui: u32, mode: u32, jump: bool) -> Result<bool, UiStateError> {
        self.set_ui_from(ui, mode, jump, None)
    }

    /// [`Self::set_ui`] for a panel's `SetUi` output: the close hook of
    /// `own` sends nothing (the panel's outputs carry its message); the
    /// other states the gate closes run theirs in full.
    fn set_ui_from(
        &mut self,
        ui: u32,
        mode: u32,
        jump: bool,
        own: Option<u8>,
    ) -> Result<bool, UiStateError> {
        let start = self.outcome.effects.len();
        let r = {
            let mut sh = self.shared.borrow_mut();
            let mut env = sh.gate_env();
            let r = sh
                .states
                .set(ui, mode, jump, &mut env, &mut self.outcome.effects);
            // The Esc menu always reopens on its first page.
            if ui == u32::from(esc_menu::ESC_PANEL.0) {
                sh.esc.menu.screen = sh.config.screen;
                sh.esc.menu.open();
                sh.esc.controls = None;
            }
            r
        };
        self.track_key_mode(start);
        let closed: Vec<u8> = self.outcome.effects[start..]
            .iter()
            .filter_map(|e| match e {
                UiEffect::Closed(u) => Some(*u),
                _ => None,
            })
            .collect();
        for u in closed {
            self.close_hook(u, own != Some(u));
        }
        r
    }

    /// The key mode after the open / close hooks of `effects[start..]`
    /// (`key_mode_for`), in effect order.
    fn track_key_mode(&mut self, start: usize) {
        use crate::controls::original::{key_mode_for, KeyModeEvent};
        let events: Vec<KeyModeEvent> = self.outcome.effects[start..]
            .iter()
            .filter_map(|e| match e {
                UiEffect::Opened(u) => Some(KeyModeEvent::UiOpen(*u)),
                UiEffect::Closed(u) => Some(KeyModeEvent::UiClose(*u)),
                _ => None,
            })
            .collect();
        for ev in events {
            let open = |u: u8| self.shared.borrow().states.is_open(u);
            if let Some(m) = key_mode_for(ev, &open) {
                self.key_mode = m;
            }
        }
    }

    /// The key mode now: 0 while the key-config screen is open (it opens in
    /// mode 0, key-up not kept), else the hooks' mode.
    pub fn key_mode(&self) -> u8 {
        if self.controls_open() {
            0
        } else {
            self.key_mode.0
        }
    }

    /// Whether the last UI draw ended with the held item's graphic, the
    /// cursor of a player holding an item: the hardware cursor shows it.
    pub fn cursor_item_drawn(&self) -> bool {
        self.shared.borrow().cursor_item_drawn.get()
    }

    /// The cursor jump the pending effects ask for
    /// ([`UiEffect::CursorX`]): the frame position `(x, mouse y)` of the last
    /// one, which also becomes the UI's mouse. Leaves the effects in place.
    pub fn take_cursor_warp(&mut self) -> Option<Point> {
        let x = self.outcome.effects.iter().rev().find_map(|e| match e {
            UiEffect::CursorX(x) => Some(*x),
            _ => None,
        })?;
        let mut sh = self.shared.borrow_mut();
        sh.mouse.x = x;
        Some(Point::new(x + FrameSize::play().ui_left(), sh.mouse.y))
    }

    /// Whether an input reset was asked for since the last
    /// call: the host
    /// applies it to the world clicks (`ClickState::input_reset`).
    pub fn take_input_reset(&mut self) -> bool {
        std::mem::take(&mut self.shared.borrow_mut().input_reset)
    }

    /// The effects and sounds since the last call.
    pub fn take_outcome(&mut self) -> UiOutcome {
        std::mem::take(&mut self.outcome)
    }

    /// The sound requests since the last call (the rest of the outcome
    /// stays).
    pub fn take_sounds(&mut self) -> Vec<SoundRequest> {
        std::mem::take(&mut self.outcome.sounds)
    }

    /// The overlay calls on units since the last call, in call order
    /// (`msg_ui::OverlayCall`), for the world view's effect layer.
    pub fn take_overlay_calls(&mut self) -> Vec<msg_ui::OverlayCall> {
        std::mem::take(&mut self.more_mut().overlays)
    }

    /// The skipped parts since the last call (the rest of the outcome
    /// stays).
    pub fn take_skipped(&mut self) -> Vec<&'static str> {
        std::mem::take(&mut self.outcome.skipped)
    }
}

/// The UI state a D2RUST hotkey action toggles (the action names are the
/// D2RUST controls' `Action`s).
pub fn hotkey_state(a: ActionId) -> Option<u8> {
    let i = usize::from(a.0);
    [
        (Action::ToggleInventory, UI_INVENTORY),
        (Action::ToggleCharacter, UI_CHARACTER),
        (Action::ToggleSkillTree, UI_SKILLTREE),
        (Action::ToggleQuests, quest_log_ui::UI_QUEST_SCREEN),
        // Command 3: SetUIState(0x18, toggle, 0).
        (Action::ToggleMessageLog, 0x18),
    ]
    .into_iter()
    .find(|(action, _)| action.index() == i)
    .map(|(_, ui)| ui)
}

/// A ground item's label: its name text and colour, as the item tip
/// names it for the local player. `None` when the item's bits are not
/// known or do not decode.
fn ground_label<'a>(
    tips: &item_tip::ItemTips,
    world: &ClientWorld,
    me: Option<&'a super::item_tip_world::WorldUnit<'a>>,
    v: &crate::bridge::items::ItemView,
) -> Option<(Vec<u16>, u16)> {
    let stream = crate::bridge::items::stream(world, v.key)?;
    let ctx = super::item_tip_world::hover_ctx(tips, world, me, v);
    tips.label_name(stream, &ctx)
}

/// A press or release of the left button (the panel handlers' mouse down / up);
/// other buttons are consumed inside an area and do nothing (which button means
/// what is the key bindings' business).
fn left(e: UiEvent) -> Option<(bool, Point)> {
    match e {
        UiEvent::Press {
            button: PointerButton::Left,
            at,
        } => Some((true, at)),
        UiEvent::Release {
            button: PointerButton::Left,
            at,
        } => Some((false, at)),
        _ => None,
    }
}

fn is_click(e: UiEvent) -> bool {
    matches!(e, UiEvent::Press { .. } | UiEvent::Release { .. })
}

const EMPTY: Rect = Rect::new(0, 0, 0, 0);

/// The mini panel's state.
const UI_MINI_PANEL: u8 = 0x15;

/// Inventory (ui 1): art, the gold line and gold button (The local player's
/// full stat 14, drawn with the fonts bound as the character values are) and
/// the close button.
struct InventoryUi {
    sh: SharedRef,
    shop: shop_ui::SharedShop,
    panel: InventoryPanel,
}

impl Panel for InventoryUi {
    fn ui_elements(&self, ctx: &UiCtx) -> Vec<super::ui_ids::UiElement> {
        let sh = self.sh.borrow();
        let Some(area) = sh.right_area() else {
            return Vec::new();
        };
        sh.items
            .layout(Facts::of(ctx.world).class, &sh.config.screen)
            .map_or_else(Vec::new, |l| l.ui_elements())
            .into_iter()
            .filter(|e| area.contains(e.at))
            .collect()
    }

    fn id(&self) -> PanelId {
        PanelId(u16::from(UI_INVENTORY))
    }

    fn rect(&self) -> Rect {
        self.sh.borrow().right_area().unwrap_or(EMPTY)
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let sh = self.sh.borrow();
        // The total, layer 0. Font16 is drawn only with the fonts bound (their
        // DC6 are then in the frame's assets).
        let gold = match (&sh.fonts, local_player(ctx.world)) {
            (Some(_), Some((key, _))) => Some(ctx.world.total(key, inv_gold::STAT_GOLD, 0)),
            _ => None,
        };
        // The gold button's pressed flag is the gold dialog module's.
        let panel = InventoryPanel {
            gold_pressed: sh.gold.buttons.inv_pressed,
            ..self.panel
        };
        panel.draw_art(&sh.tables, &sh.env(), out);
        let class = Facts::of(ctx.world).class;
        // The empty equipment slots' pictures, under the items.
        sh.items
            .draw_equip_backgrounds(ctx.world, &sh.tables.files, class, &sh.config.screen, out);
        if let Some(l) = sh.items.layout(class, &sh.config.screen) {
            sh.items
                .draw_items(ctx.world, &sh.tables.files, &l, sh.mouse, out);
            // The placement tint after the items' own
            // tints and cels (`w2-12-inv-tint-ama`: the potion's tint and
            // cel, then the carried sword's three cell boxes).
            sh.items.draw_placement_tint(
                ctx.world,
                &sh.tables.files,
                &l.grid,
                0,
                sh.mouse,
                sh.config.screen.h,
                out,
            );
        }
        // The gold line, gold button and close button after the
        // items (`a1-panel-cube` rows 41–45).
        panel.draw_tail(&sh.tables, &sh.env(), gold, out);
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        if !is_click(e) {
            return UiResponse::Ignored;
        }
        let mut sh = self.sh.borrow_mut();
        let s = sh.config.screen;
        if let UiEvent::Press {
            button: PointerButton::Right,
            at,
        } = e
        {
            let class = Facts::of(ctx.world).class;
            if let Some(l) = sh.items.layout(class, &s) {
                let out = sh.items.right_press(ctx.world, &sh.tables.files, &l, at);
                sh.outputs.extend(out);
            }
            return UiResponse::Consumed;
        }
        match left(e) {
            Some((true, at)) => {
                self.panel.press(&sh.tables, &s, at);
                let class = Facts::of(ctx.world).class;
                if let Some(l) = sh.items.layout(class, &s) {
                    // With the shop up a player item is sold or repaired
                    // (the shop's grid and body-location callers), not
                    // picked up.
                    let under = sh.items.item_under(ctx.world, &l.grid, &l, at);
                    let taken = shop_ui::ShopUi::player_item_click(
                        &mut sh,
                        &mut self.shop.borrow_mut(),
                        ctx.world,
                        ctx.strings,
                        under.as_ref(),
                    );
                    if taken {
                        return UiResponse::Consumed;
                    }
                    let out = sh.items.press(ctx.world, &sh.tables.files, &l, at);
                    sh.outputs.extend(out);
                }
            }
            Some((false, at)) => {
                let out = self.panel.release(&sh.tables, &s, at);
                sh.outputs.extend(out);
            }
            None => {
                // Right press: use the item under the mouse (the item use
                // inputs).
                if let UiEvent::Press {
                    button: PointerButton::Right,
                    at,
                } = e
                {
                    let class = Facts::of(ctx.world).class;
                    if let Some(l) = sh.items.layout(class, &s) {
                        let out = sh.items.use_press(ctx.world, &l, at);
                        sh.outputs.extend(out);
                    }
                }
            }
        }
        UiResponse::Consumed
    }
}

/// The skill tree's view of the model: the class, the icon file, the free
/// points and the entries joined from the skill rows and the local
/// player's list ([`skill_tree_ui::entries`]).
struct ModelSkillTree {
    class: Option<u8>,
    icon_file: Option<u32>,
    free_points: i32,
    entries: Vec<SkillEntry>,
}

impl ModelSkillTree {
    fn of(sh: &Shared, world: &ClientWorld) -> Self {
        let class = class_u8(sh.facts.class);
        ModelSkillTree {
            class,
            icon_file: class
                .and_then(skill_tree_ui::icon_file_name)
                .and_then(|n| sh.tables.files.id(&n)),
            free_points: world.local().map_or(0, |u| u.stat(5)),
            entries: class
                .map(|c| skill_tree_ui::entries(&sh.skill_tree, world, c))
                .unwrap_or_default(),
        }
    }
}

impl SkillTreeView for ModelSkillTree {
    fn class(&self) -> Option<u8> {
        self.class
    }
    fn icon_file(&self) -> Option<u32> {
        self.icon_file
    }
    fn free_points(&self) -> i32 {
        self.free_points
    }
    fn flag_mask(&self) -> u8 {
        skill_tree_ui::FLAG_INGAME
    }
    fn skills(&self) -> &[SkillEntry] {
        &self.entries
    }
}

fn class_u8(class: Option<u32>) -> Option<u8> {
    class.and_then(|c| u8::try_from(c).ok()).filter(|&c| c < 7)
}

/// Skill tree (ui 4): back art per class and tab, tabs, close, the
/// free-points number and the close / tab tool tips.
struct SkillTreeUi {
    sh: SharedRef,
    panel: SkillTreePanel,
}

/// The `uninterruptable` state: the free-points number in colour 1.
const STATE_UNINTERRUPTABLE: u8 = 54;

impl Panel for SkillTreeUi {
    fn id(&self) -> PanelId {
        PanelId(u16::from(UI_SKILLTREE))
    }

    /// The right half above the control panel: the tab-1 band reaches
    /// y = H − 49, past the inventory area's bottom; the press
    /// and release handlers decide what they consume.
    fn rect(&self) -> Rect {
        let sh = self.sh.borrow();
        if sh.right_area().is_none() {
            return EMPTY;
        }
        let s = sh.config.screen;
        let (x0, x1) = (s.right_start(), s.frame_right());
        Rect::new(x0, 0, (x1 - x0) as u16, (s.h - 47) as u16)
    }

    fn ui_elements(&self, ctx: &UiCtx) -> Vec<super::ui_ids::UiElement> {
        let sh = self.sh.borrow();
        if ctx.world.local().is_none() {
            return Vec::new();
        }
        self.panel
            .ui_elements(&sh.config.screen, &ModelSkillTree::of(&sh, ctx.world))
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let sh = self.sh.borrow();
        let view = ModelSkillTree::of(&sh, ctx.world);
        let uninterruptable = ctx
            .world
            .local()
            .is_some_and(|u| u.states.contains(&STATE_UNINTERRUPTABLE));
        let mut captions: Vec<super::draw::UiDraw> = Vec::new();
        if let (Some(class), Some(fonts)) = (view.class(), sh.fonts.as_ref()) {
            let s = sh.config.screen;
            let (a, b) = super::panels::skilltree::caption_span(&s);
            let font = super::panels::skilltree::FONT16;
            for (id, k) in super::panels::skilltree::caption_lines(class) {
                if let Some(t) = ctx.strings.get_id(id) {
                    let t = t.to_vec();
                    if let Some(w) = fonts.width(font, &t) {
                        captions.push(text(t, centered_in(a, b, w), s.h + s.sy() + k, font, 0));
                    }
                }
            }
            // No free points: the "0".
            if view.free_points() < 1 {
                let (a, b, y) = super::panels::skilltree::zero_points_span(&s);
                let t: Vec<u16> = vec![u16::from(b'0')];
                if let Some(w) = fonts.width(font, &t) {
                    captions.push(text(t, centered_in(a, b, w), y, font, 0));
                }
            }
        }
        self.panel.draw_with_points(
            &sh.tables,
            &sh.env(),
            &view,
            sh.mouse,
            Some(uninterruptable),
            &captions,
            out,
        );
        let s = sh.config.screen;
        let tab = self.panel.tab;
        // The hovered icon's description, drawn by the pop-up layer.
        sh.skill_hover.set(
            self.panel
                .hovered(&s, &view, sh.mouse)
                .map(|(skill, level, at)| skill_hover::SkillHover {
                    skill,
                    level,
                    at,
                    above: false,
                    // For a state −1 icon, else.
                    learnable: view
                        .skills()
                        .iter()
                        .any(|e| e.skill == skill && e.learnable),
                }),
        );
        // `strClose` queued (the pop-up text) in the close
        // rectangle.
        if let Some(class) = view.class() {
            if super::panels::skilltree::close_hit(&s, class, tab, sh.mouse) {
                if let (Some(at), Some(t)) = (
                    super::panels::skilltree::close_tool_tip_at(&s, class, tab),
                    ctx.strings.get_id(cube_ui::STR_CLOSE),
                ) {
                    let fonts = sh.fonts.as_ref();
                    hud_tips::push_popup(t.to_vec(), at, 0, true, (s.w, s.h), fonts, out);
                }
            }
        }
        // The tab tool tip, at once: the rectangle (colour (0, 0,
        // 0) nearest: palette index 0, mode 6) and the text, Font16,
        // left-aligned.
        if let Some(tip) = super::panels::skilltree::tab_tool_tip(&s, tab, sh.mouse) {
            if let Some(t) = ctx.strings.get_id(tip.string) {
                let (x0, y0, x1, y1) = tip.rect;
                out.push(super::draw::UiDraw::Rect(super::draw::RectRequest {
                    x0,
                    y0,
                    x1,
                    y1,
                    color: 0,
                    mode: 6,
                }));
                out.push(text(
                    t.to_vec(),
                    tip.text_at.x,
                    tip.text_at.y,
                    super::panels::skilltree::FONT16,
                    0,
                ));
            }
        }
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        if !is_click(e) {
            return UiResponse::Ignored;
        }
        let mut sh = self.sh.borrow_mut();
        let env = sh.env();
        let view = ModelSkillTree::of(&sh, ctx.world);
        let s = sh.config.screen;
        let input = |at: Point| super::panels::skilltree::InputCtx {
            over_belt: sh
                .hud
                .belt
                .over(ctx.world, (s.w, s.h), s.res2(), (at.x, at.y)),
            has_player: ctx.world.local().is_some(),
        };
        let consumed = match left(e) {
            Some((true, at)) => {
                let d = self.panel.mouse_down_ctx(&env, &view, at, input(at));
                if d.cursor_press {
                    cursor_ui::cursor_press(&sh, ctx.world, at);
                }
                sh.outputs.extend(d.out);
                d.consumed
            }
            Some((false, at)) => {
                let r = self.panel.mouse_up_ctx(&env, &view, at, input(at));
                sh.outputs.extend(r.out);
                // X ≤ W / 2 is not consumed.
                r.consumed
            }
            None => true,
        };
        if consumed {
            UiResponse::Consumed
        } else {
            UiResponse::Ignored
        }
    }
}

/// Character (ui 2): art and close button; with the fonts and a
/// local player also the stat block, values and name line
/// ([`ModelCharacter`]; the rest is in `PENDING`).
struct CharacterUi {
    sh: SharedRef,
    panel: CharacterPanel,
}

const CHARACTER: PanelKey = PanelKey::Ui(UI_CHARACTER);

/// The fonts the panels measure: Font8 (0), Font16 (1), Font6 (6) of the
/// character panel, FontFormal11
/// (8) of the quest log's description pane and FontInGameChat (13) of the
/// screen messages.
pub const CHARACTER_FONTS: [u16; 6] = [0, 1, 5, 6, 8, 13];

/// Glyph widths of the loaded font tables: width A for
/// centering, max width for the stat value pop-up width.
#[derive(Clone, Debug, Default)]
pub struct FontMeasure {
    tables: BTreeMap<u16, FontTable>,
}

impl FontMeasure {
    /// Font `id`'s table.
    pub fn insert(&mut self, id: u16, table: FontTable) {
        self.tables.insert(id, table);
    }

    /// Reads the `.tbl` of each font id from `source`. A font in no archive or
    /// that does not parse is an error.
    pub fn load(source: &dyn FileSource, ids: &[u16]) -> Result<Self, String> {
        let mut m = FontMeasure::default();
        for &id in ids {
            let info = super::font_info(id).ok_or(format!("font id {id} is not 0–13"))?;
            let archive = info.tbl_path.replace('/', "\\");
            let table = crate::assets::path::read_font_table(source, &archive)
                .ok_or(format!("{archive}: in no archive"))?
                .map_err(|e| format!("{archive}: {e}"))?;
            m.insert(id, table);
        }
        Ok(m)
    }

    /// `Wrap(text, max)`: the lines; `None` without
    /// the font.
    pub fn wrap(&self, font: u16, text: &[u16], max: i32) -> Option<Vec<Vec<u16>>> {
        let g = super::text::GlyphLookup::new(self.tables.get(&font)?);
        let lines = super::text::wrap(&g, text, max).ok()?;
        Some(lines.into_iter().map(<[u16]>::to_vec).collect())
    }

    /// Text height of `text` in `font`;
    /// `None` without the font.
    pub fn text_height(&self, font: u16, text: &[u16]) -> Option<i32> {
        let g = super::text::GlyphLookup::new(self.tables.get(&font)?);
        Some(super::text::text_height(&g, text))
    }

    /// The font's line height (byte 10 of the `.tbl`); `None` without the font.
    pub fn font_height(&self, font: u16) -> Option<i32> {
        Some(super::text::GlyphLookup::new(self.tables.get(&font)?).height())
    }

    /// Width A of `text`; `None` without the font.
    pub fn width_a(&self, font: u16, text: &[u16]) -> Option<i32> {
        TextMeasure::width(self, font, text)
    }

    /// Width C of the whole `text`.
    pub fn width_c(&self, font: u16, text: &[u16]) -> Option<i32> {
        let g = super::text::GlyphLookup::new(self.tables.get(&font)?);
        super::text::width_c(&g, text, 0, text.len()).ok()
    }

    /// Max width of `text` in font `font`; `None`
    /// without the font or for a code with no glyph record.
    pub fn max_width(&self, font: u16, text: &[u16]) -> Option<i32> {
        let g = super::text::GlyphLookup::new(self.tables.get(&font)?);
        super::text::max_width(&g, text).ok()
    }
}

impl FontMeasure {
    /// The pop-up text draw of `text` at `at` in font
    /// `font`; `None` without the font or
    /// for a code with no glyph record.
    pub fn popup(
        &self,
        font: u16,
        text: &[u16],
        at: Point,
        centre: bool,
        screen: (i32, i32),
    ) -> Option<super::text::FramedText> {
        let g = super::text::GlyphLookup::new(self.tables.get(&font)?);
        super::text::popup_text(&g, text, at, centre, screen).ok()
    }
}

impl FontMeasure {
    /// The measure of a Show Items label in font `font`; `None` without the
    /// font or for a code with no glyph record.
    pub fn label_metrics(&self, font: u16, text: &[u16]) -> Option<super::show_items::Metrics> {
        let g = super::text::GlyphLookup::new(self.tables.get(&font)?);
        Some(super::show_items::Metrics {
            width: super::text::max_width(&g, text).ok()?,
            height: super::text::text_height(&g, text),
            font_height: g.height(),
        })
    }
}

impl TextMeasure for FontMeasure {
    /// Width A, the centering width.
    fn width(&self, font: u16, text: &[u16]) -> Option<i32> {
        let g = super::text::GlyphLookup::new(self.tables.get(&font)?);
        super::text::width_a(&g, text).ok()
    }
}

/// The character panel's view of the client model: the local player's full
/// values (`total`) and bases (`base`), layer 0.
pub struct ModelCharacter<'a> {
    pub world: &'a ClientWorld,
    pub key: UnitKey,
    /// The popup width's font table (Font16).
    pub fonts: &'a FontMeasure,
    /// `difficultylevels` `ResistPenalty` by difficulty (expansion game).
    pub penalties: &'a [i32],
    /// The extra tables and the `experience` rows ([`char_feed`]).
    pub chars: &'a super::char_feed::CharTables,
    pub experience: &'a [[u32; 7]],
}

impl ModelCharacter<'_> {
    /// The defense line of the panel: the panel's stat 31 getter, which
    /// computes it instead of reading the
    /// stat. `base = armorclass(31) + dexterity(2) / 4` (truncating), the
    /// armor percents (stats 171 and 16) of `base` (negative bases use the
    /// negated divisor), then the override percent (stat 182) of the sum.
    /// The Holy Shield term is not modelled here (state 101 is rare on the
    /// character screen; it needs the skill formula).
    fn defense(&self) -> i32 {
        let w = self.world;
        let t = |id| w.total(self.key, id, 0);
        let base = t(31).wrapping_add(t(2) / 4);
        let p = t(171).wrapping_add(t(16));
        let bonus = if base > 0 {
            p.wrapping_mul(base) / 100
        } else {
            p.wrapping_mul(base) / -100
        };
        let total = base.wrapping_add(bonus);
        let ovr = t(182);
        if ovr != 0 {
            total.wrapping_add(total.wrapping_mul(ovr) / 100)
        } else {
            total
        }
    }
}

impl CharacterView for ModelCharacter<'_> {
    fn stat(&self, id: u16) -> i32 {
        if id == 31 {
            return self.defense();
        }
        self.world.total(self.key, id, 0)
    }

    fn base(&self, id: u16) -> i32 {
        self.world.base(self.key, id, 0)
    }

    fn alive(&self) -> bool {
        self.world
            .units
            .get(&self.key)
            .is_some_and(|u| !u.is_dead())
    }

    /// English (the only locale in scope).
    fn language(&self) -> u8 {
        0
    }

    /// As subtracted by the panel: the negated
    /// adjustment of the client's difficulty.
    fn resist_penalty(&self) -> i32 {
        let w = self.world;
        -char_inputs::resist_value(
            0,
            w.expansion != 0,
            usize::from(w.difficulty),
            self.penalties,
        )
    }

    /// The state tests by the `states` flags.
    fn resist_effect(&self, id: u16) -> ResistEffect {
        match self.world.units.get(&self.key) {
            Some(u) => self.chars.resist_effect(&u.states, id),
            None => ResistEffect::None,
        }
    }

    fn defense_color(&self) -> Option<u16> {
        self.chars
            .defense_color(&self.world.units.get(&self.key)?.states)
    }

    fn next_level(&self) -> Option<u32> {
        let u = self.world.units.get(&self.key)?;
        let level = u32::try_from(self.base(12)).unwrap_or(0);
        let stat30 = u32::try_from(self.stat(30)).unwrap_or(0);
        super::char_feed::next_level(self.experience, u.class, level, stat30)
    }

    /// The max width of the value in Font16.
    fn popup_width(&self, text: &[u16]) -> Option<i32> {
        self.fonts.max_width(1, text)
    }
}

/// The local player's key and 0x59 name (up to its NUL), when the model
/// has a local player unit.
pub(super) fn local_player(world: &ClientWorld) -> Option<(UnitKey, &[u8])> {
    let u = world.local().filter(|u| u.key.unit_type == PLAYER)?;
    let name: &[u8] = match &u.kind {
        KindData::Player(p) => {
            let n = p.name.iter().position(|&b| b == 0).unwrap_or(16);
            &p.name[..n]
        }
        _ => &[],
    };
    Some((u.key, name))
}

/// The name line, its font by code-point count,
/// centered in [`sx + 13`, `sx + 160`] at y `H + sy − 455`, color 0.
/// Not drawn when the font has no width for it.
pub fn name_line(
    name: &[u8],
    s: &super::layout::Screen,
    measure: &dyn TextMeasure,
    out: &mut dyn UiDrawSink,
) {
    let font = char_details::name_font(char_details::name_code_points(name));
    // → u16 (128 units): each byte a unit (the D2RUST names
    // are ASCII, `play --new`; a save's name bytes as Latin-1).
    let s16: Vec<u16> = name.iter().take(128).map(|&b| u16::from(b)).collect();
    let Some(w) = measure.width(font, &s16) else {
        return;
    };
    let (a, b) = char_details::name_span(s);
    out.push(text(
        s16,
        centered_in(a, b, w),
        char_details::line_y(s),
        font,
        0,
    ));
}

impl CharacterUi {
    /// Art and close button only (no fonts or no local player).
    /// The panel without the close tip (`Panel::draw`).
    fn draw_panel(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let sh = self.sh.borrow();
        let expansion = ctx.world.expansion != 0;
        let penalties = match (&sh.resist_penalties, expansion) {
            (Some(p), _) => Some(p.as_slice()),
            // A classic game's penalty is fixed.
            (None, false) => Some(&[][..]),
            (None, true) => None,
        };
        let (Some(fonts), Some(penalties), Some((key, name))) =
            (&sh.fonts, penalties, local_player(ctx.world))
        else {
            self.draw_static(&sh, out);
            return;
        };
        let view = ModelCharacter {
            world: ctx.world,
            key,
            fonts,
            penalties,
            chars: &sh.char_tables,
            experience: &sh.hud.tables.experience,
        };
        let env = sh.env();
        // Draw order (`panels/character.rs` `draw_staged`): labels, the skill
        // damage blocks, the close button, the class and name lines, values.
        self.panel.draw_staged(
            &sh.tables,
            &env,
            &view,
            fonts,
            ctx.strings,
            out,
            &mut |stage, out| match stage {
                character::Stage::Labels => super::char_feed::damage_block(
                    ctx.world,
                    key,
                    &sh.char_tables,
                    ctx.strings,
                    &env.screen,
                    fonts,
                    out,
                ),
                character::Stage::Close => {
                    if let Some(u) = ctx.world.units.get(&key) {
                        super::char_feed::class_line(
                            u.class,
                            &sh.char_tables,
                            ctx.strings,
                            &env.screen,
                            fonts,
                            out,
                        );
                    }
                    name_line(name, &env.screen, fonts, out);
                }
            },
        );
    }

    fn draw_static(&self, sh: &Shared, out: &mut dyn UiDrawSink) {
        let env = sh.env();
        let t = &sh.tables;
        // Draw rows in file order: the close rows with the close
        // button's pressed flag; rows needing panel words (stat points)
        // do not apply without them.
        for r in t.rows(CHARACTER) {
            if r.kind != RowKind::Draw {
                continue;
            }
            let pressed = r.item == "close" && self.panel.close_pressed;
            let cond = env.cond(pressed, &no_extra);
            let one = |x: &super::layout::LayoutRow| std::ptr::eq(x, r);
            emit_static_draws(t, CHARACTER, &cond, None, &one, out);
        }
    }
}

impl Panel for CharacterUi {
    fn id(&self) -> PanelId {
        PanelId(u16::from(UI_CHARACTER))
    }

    /// X in [`sx`, `W / 2 − 1`], y in [`sy`, `H + sy − 49`].
    fn rect(&self) -> Rect {
        character::area(&self.sh.borrow().config.screen)
    }

    fn ui_elements(&self, ctx: &UiCtx) -> Vec<super::ui_ids::UiElement> {
        let sh = self.sh.borrow();
        let statpts = match (&sh.fonts, local_player(ctx.world)) {
            (Some(_), Some((key, _))) => ctx.world.base(key, STAT_STATPTS, 0),
            _ => 0,
        };
        self.panel
            .ui_elements(&sh.tables, &sh.config.screen, statpts)
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        self.draw_panel(ctx, out);
        // `strClose` at (sx + 143, H + sy − 95) while the mouse is
        // in the close rectangle, as the pop-up text. D2RUST's own,
        // unverified: centred (the x is the button's middle; the call's
        // centre flag is not known).
        let sh = self.sh.borrow();
        let s = sh.config.screen;
        let over = super::panels::character::close_rect(&sh.tables, &s)
            .is_some_and(|r| r.contains(sh.mouse));
        if let (true, Some(t)) = (over, ctx.strings.get_id(cube_ui::STR_CLOSE)) {
            let at = Point::new(s.sx() + 143, s.h + s.sy() - 95);
            hud_tips::push_popup(t.to_vec(), at, 0, true, (s.w, s.h), sh.fonts.as_ref(), out);
        }
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    fn event(&mut self, e: UiEvent, ctx: &UiCtx) -> UiResponse {
        if !is_click(e) {
            return UiResponse::Ignored;
        }
        let mut sh = self.sh.borrow_mut();
        let s = sh.config.screen;
        // The base stat points gate the add
        // buttons; the add rows are drawn only with the fonts bound.
        let statpts = match (&sh.fonts, local_player(ctx.world)) {
            (Some(_), Some((key, _))) => ctx.world.base(key, STAT_STATPTS, 0),
            _ => 0,
        };
        match left(e) {
            Some((true, at)) => {
                if self.panel.press(&sh.tables, &s, at, statpts) {
                    cursor_ui::cursor_press(&sh, ctx.world, at);
                }
            }
            Some((false, at)) => {
                // Shift is the host's per-frame flag (`set_shift`): all points.
                let shift = sh.items.shift;
                let out = self.panel.release(&sh.tables, &s, at, shift, statpts);
                sh.outputs.extend(out);
            }
            None => {}
        }
        UiResponse::Consumed
    }
}

/// Border and control panel base, drawn every frame.
/// Its area is empty: it takes no event.
struct BorderUi {
    sh: SharedRef,
}

impl Panel for BorderUi {
    fn id(&self) -> PanelId {
        BORDER_PANEL
    }

    fn rect(&self) -> Rect {
        EMPTY
    }

    fn draw(&self, _ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let sh = self.sh.borrow();
        // The UI box sits `ui_left` pixels into a wider frame: what covers
        // the whole screen reaches from −ui_left to W + ui_left.
        let ui_left = FrameSize::play().ui_left();
        // The help screen dims the world behind it, before the control
        // panel; 800 × 600 layout only.
        if sh.states.is_open(help_screen::STATE) && sh.config.screen.res2() {
            let s = sh.config.screen;
            out.push(help_screen::dim_box(-ui_left, s.w + 2 * ui_left, s.h));
        }
        draw_border_and_ctrlpnl(&sh.tables, &sh.env(), out);
        draw_ctrlpnl_sides(&sh.tables, &sh.env(), ui_left, out);
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    fn event(&mut self, _e: UiEvent, _ctx: &UiCtx) -> UiResponse {
        UiResponse::Ignored
    }
}

/// The hover tips and the cursor item (drawn after the UI pass): after the
/// border and control panel, the HUD overlays and every other panel, so a held
/// item or a tip over the control panel stays on top. Its area is empty: it
/// takes no event.
struct TopUi {
    sh: SharedRef,
    shop: shop_ui::SharedShop,
}

impl Panel for TopUi {
    fn id(&self) -> PanelId {
        TOP_PANEL
    }

    fn rect(&self) -> Rect {
        EMPTY
    }

    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        let sh = self.sh.borrow();
        // The belt item's hover text (`hud_belt`) as the r14 pop-up, in the
        // belt's font 1.
        if let Some(t) = sh
            .items
            .tips
            .as_ref()
            .and_then(|tips| sh.hud.belt.hover_tip(ctx.world, tips))
        {
            let (w, h) = (sh.config.screen.w, sh.config.screen.h);
            hud_tips::push_popup(
                t.text,
                Point::new(t.x, t.y),
                u16::from(t.color),
                t.centered,
                (w, h),
                sh.fonts.as_ref(),
                out,
            );
        }
        // The item tool tip over everything (`item_tip`), while the inventory
        // family is drawn (with the stash, the cube or the shop too, not only
        // flag 1).
        let family = super::root::INVENTORY_FAMILY
            .iter()
            .any(|&u| sh.states.is_open(u));
        if family {
            let class = Facts::of(ctx.world).class;
            let lines = sh.items.hover_lines(
                ctx.world,
                &sh.tables.files,
                &sh.config.screen,
                class,
                sh.mouse,
            );
            let (w, h) = (sh.config.screen.w, sh.config.screen.h);
            let anchor = sh
                .items
                .hover_anchor(
                    ctx.world,
                    &sh.tables.files,
                    &sh.config.screen,
                    class,
                    sh.mouse,
                )
                .unwrap_or_else(|| item_tip::TipAnchor::at_point(sh.mouse));
            item_tip::draw_tip_at(&lines, anchor, (w, h), sh.fonts.as_ref(), out);
        }
        // The hovered skill icon's description (`skill_hover`). Either
        // pop-up replaces the world hover label.
        sh.popup_taken.set(false);
        if let Some(hover) = sh.skill_hover.take() {
            sh.popup_taken.set(true);
            // The box reads the local player's model and the skills
            // tables (`skill_hover_lines::ModelInputs`); none without them.
            let tables = sh.char_tables.desc.skills.as_deref();
            if let (Some(t), Some(key)) = (tables, ctx.world.local_player) {
                let mut inputs = skill_hover_lines::ModelInputs::new(
                    ctx.world,
                    key,
                    &sh.char_tables.desc,
                    t,
                    ctx.strings,
                    &sh.hud.tables.hover,
                );
                let (w, h) = (sh.config.screen.w, sh.config.screen.h);
                skill_hover::draw(&hover, &mut inputs, sh.fonts.as_ref(), (w, h), out);
            }
        }
        if let Some(hover) = sh.button_hover.take() {
            sh.popup_taken.set(true);
            // A skill without a `skilldesc` record still pops up.
            let text = sh.hud.tables.descs.get(&hover.skill).copied();
            let (w, h) = (sh.config.screen.w, sh.config.screen.h);
            skill_hover::draw_button(&hover, ctx.strings, text, sh.fonts.as_ref(), (w, h), out);
        }
        // The store item's tip (the shop is part of the inventory family).
        shop_ui::draw_hover_tip(&sh, &self.shop.borrow(), ctx, out);
        // The cursor last: the item's graphic or the type's cel
        // (`cursor_ui`).
        cursor_ui::draw(&sh, ctx, out);
    }

    fn hit(&self, _p: Point) -> Option<WidgetId> {
        None
    }

    fn event(&mut self, _e: UiEvent, _ctx: &UiCtx) -> UiResponse {
        UiResponse::Ignored
    }
}

#[path = "controls_host.rs"]
pub mod controls_host;
#[path = "esc_art.rs"]
pub mod esc_art;
#[path = "esc_menu.rs"]
pub mod esc_menu;
#[path = "game_messages.rs"]
pub mod game_messages;
#[path = "gold_dialog.rs"]
pub mod gold_dialog;
#[path = "hud.rs"]
pub mod hud;
#[path = "options_menu.rs"]
pub mod options_menu;
#[path = "overhead_ui.rs"]
pub mod overhead_ui;

#[path = "help_screen.rs"]
pub mod help_screen;
#[path = "hud_belt.rs"]
pub mod hud_belt;
#[path = "hud_hotkeys.rs"]
pub mod hud_hotkeys;
#[path = "hud_tips.rs"]
pub mod hud_tips;

#[path = "cube_ui.rs"]
pub(super) mod cube_ui;
#[path = "cursor_ui.rs"]
pub mod cursor_ui;
#[path = "msg_log_ui.rs"]
pub mod msg_log_ui;
#[cfg(test)]
#[path = "msg_log_ui_tests.rs"]
mod msg_log_ui_tests;
#[path = "msg_ui.rs"]
pub mod msg_ui;
#[path = "npc_box.rs"]
pub mod npc_box;
#[path = "npc_talk.rs"]
pub mod npc_talk;
#[path = "skill_hover.rs"]
pub mod skill_hover;
#[path = "skill_hover_lines.rs"]
pub mod skill_hover_lines;
use cursor_ui::cursor_files;
#[path = "quest_log_ui.rs"]
pub mod quest_log_ui;
#[cfg(test)]
#[path = "quest_log_ui_tests.rs"]
mod quest_log_ui_tests;
#[path = "shop_ui.rs"]
pub mod shop_ui;
#[path = "stash_ui.rs"]
pub(super) mod stash_ui;
#[path = "waypoint_ui.rs"]
pub mod waypoint_ui;
pub use msg_ui::{
    ChatAction, IntroEntry, MsgUiMore, MsgUiState, NpcTextList, OverheadText, WaypointMenuState,
};
pub use shop_ui::ShopPrices;
pub use waypoint_ui::WaypointOpen;

#[cfg(test)]
#[path = "original_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "char_feed_tests.rs"]
mod char_feed_tests;

#[cfg(test)]
mod character_bind_tests {
    use super::*;
    use crate::assets::path::MemorySource;
    use crate::bridge::world::{ClientUnit, PlayerData};
    use crate::ui::{ClientIntent, NoPanelRules, NoStrings, UiDraw};

    /// A synthetic `.tbl`: 256 records, record `i`
    /// of width `w`.
    pub(crate) fn tbl(w: u8) -> Vec<u8> {
        let mut d = b"Woo!".to_vec();
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&256u16.to_le_bytes());
        d.extend_from_slice(&[10, 0]);
        for i in 0..256u16 {
            d.extend_from_slice(&i.to_le_bytes());
            d.extend_from_slice(&[0, w, 10, 0, 0, 0]);
            d.extend_from_slice(&i.to_le_bytes());
            d.extend_from_slice(&[0; 4]);
        }
        d
    }

    fn fonts() -> FontMeasure {
        let mut src = MemorySource::default();
        for id in CHARACTER_FONTS {
            src.insert(crate::ui::font_info(id).unwrap().tbl_path, tbl(6));
        }
        FontMeasure::load(&src, &CHARACTER_FONTS).unwrap()
    }

    fn player(name: &[u8], expansion: bool) -> (ClientWorld, UnitKey) {
        let mut w = ClientWorld::default();
        let key = UnitKey::new(PLAYER, 1);
        let mut u = ClientUnit::new(key);
        u.class = 4;
        u.mode = 1;
        let mut p = PlayerData::default();
        p.name[..name.len()].copy_from_slice(name);
        u.kind = KindData::Player(p);
        for (stat, v) in [(0u16, 30), (12, 7), (6, 55 << 8), (7, 55 << 8), (39, 10)] {
            u.stats.insert(stat, v);
        }
        // A state list raising strength: total 35 > base 30 (blue).
        u.state_lists
            .insert(1, [((0u16, 0u16), 5)].into_iter().collect());
        w.units.insert(key, u);
        w.local_player = Some(key);
        w.expansion = u32::from(expansion);
        (w, key)
    }

    fn open(fonts: Option<FontMeasure>) -> (OriginalUi, UiRoot) {
        let config = UiConfig {
            screen: Screen::R800,
            expansion_installed: true,
        };
        let mut ui = OriginalUi::new(config, None).unwrap();
        if let Some(f) = fonts {
            ui.set_fonts(f);
        }
        let mut root = UiRoot::new(Box::new(NoPanelRules));
        ui.install(&mut root).unwrap();
        ui.set_ui(u32::from(UI_CHARACTER), 2, false).unwrap();
        root.sync_states(&ui.shared.borrow().states);
        (ui, root)
    }

    fn texts(root: &UiRoot, w: &ClientWorld) -> Vec<(String, i32, i32, u16, u16)> {
        let ctx = UiCtx {
            tick: 0,
            world: w,
            strings: &NoStrings,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        root.draw(&ctx, &mut out);
        out.iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some((
                    String::from_utf16_lossy(&t.text),
                    t.at.x,
                    t.at.y,
                    t.style.font,
                    t.style.color,
                )),
                _ => None,
            })
            .collect()
    }

    fn find<'a>(
        v: &'a [(String, i32, i32, u16, u16)],
        s: &str,
    ) -> Option<&'a (String, i32, i32, u16, u16)> {
        v.iter().find(|t| t.0 == s)
    }

    #[test]
    fn values_and_name_come_from_the_model() {
        let (w, _) = player(b"Conan", false);
        let (_ui, root) = open(Some(fonts()));
        let t = texts(&root, &w);
        let s = Screen::R800;
        let (sx, y0) = (s.sx(), s.h + s.sy());
        // Level (stat 12): `sx + 13`, w 41, y `H + sy − 421`; "7" is 6
        // wide: x = sx + 13 + ((41 − 6) >> 1).
        assert_eq!(find(&t, "7"), Some(&("7".into(), sx + 30, y0 - 421, 1, 0)));
        // Strength: total 35 over base 30 → blue (3).
        let st = find(&t, "35").expect("strength");
        assert_eq!((st.2, st.4), (y0 - 381, 3));
        // Life `>> 8`, color 0 (current life is not compared).
        assert_eq!(find(&t, "55").map(|v| v.4), Some(0));
        // Fire resist, classic Normal: no penalty.
        assert!(find(&t, "10").is_some());
        // Name line: 5 code points → Font16; 30 wide centered in
        // [sx + 13, sx + 160].
        assert_eq!(
            find(&t, "Conan"),
            Some(&("Conan".into(), sx + 13 + ((148 - 30) >> 1), y0 - 455, 1, 0))
        );
    }

    #[test]
    fn long_names_switch_font() {
        let f = fonts();
        let mut out: Vec<UiDraw> = Vec::new();
        name_line(b"ABCDEFGHIJKL", &Screen::R800, &f, &mut out);
        name_line(b"ABCDEFGHIJKLM", &Screen::R800, &f, &mut out);
        let fonts: Vec<u16> = out
            .iter()
            .map(|d| match d {
                UiDraw::Text(t) => t.style.font,
                _ => panic!("text"),
            })
            .collect();
        assert_eq!(fonts, [0, 6]);
    }

    #[test]
    fn an_expansion_game_needs_the_resist_penalties() {
        let (w, _) = player(b"Conan", true);
        let (mut ui, root) = open(Some(fonts()));
        assert!(texts(&root, &w).is_empty(), "no table: no values");
        // `ResistPenalty` 0 / −40 / −100; Normal adds 0.
        ui.set_resist_penalties(vec![0, -40, -100]);
        assert!(find(&texts(&root, &w), "10").is_some());
        let mut hell = w.clone();
        hell.difficulty = 2;
        // 10 − 100 = −90. Its color (red when shown < 0) is
        // `panels/character.rs`'s `resist_value`, which still applies the
        // Compare (handoff, findings).
        assert!(find(&texts(&root, &hell), "-90").is_some());
    }

    #[test]
    fn stat_points_show_the_box_and_spend_through_the_root() {
        let (mut w, key) = player(b"Conan", false);
        w.units.get_mut(&key).unwrap().stats.insert(STAT_STATPTS, 5);
        let (mut ui, mut root) = open(Some(fonts()));
        let t = texts(&root, &w);
        assert!(find(&t, "5").is_some(), "{t:?}");
        // Strength's add button (117, 105): b = H + sy − 480 + 105.
        let s = Screen::R800;
        let at = Point::new(s.sx() + 130, s.h + s.sy() - 480 + 105 - 10);
        let ctx = UiCtx {
            tick: 0,
            world: &w,
            strings: &NoStrings,
        };
        for e in [
            UiEvent::Press {
                button: PointerButton::Left,
                at,
            },
            UiEvent::Release {
                button: PointerButton::Left,
                at,
            },
        ] {
            ui.before_event(e, &w);
            let r = root.dispatch(e, &ctx);
            ui.after_event(&mut root, e, r).unwrap();
        }
        assert_eq!(root.intents(), &[ClientIntent(vec![0x3A, 0, 0])]);
    }

    #[test]
    fn without_fonts_only_the_art_draws() {
        let (w, _) = player(b"Conan", false);
        let (_ui, root) = open(None);
        assert!(texts(&root, &w).is_empty());
        let missing = FontMeasure::load(&MemorySource::default(), &[1]);
        assert!(missing.is_err());
    }
}

#[path = "skill_bindings.rs"]
mod skill_bindings;
pub use skill_bindings::SkillKeyResult;
