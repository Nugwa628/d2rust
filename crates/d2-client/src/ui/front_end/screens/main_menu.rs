//! The main menu: the background, the two halves of the burning logo,
//! the buttons and the version text, built in draw order. With the
//! `online` feature the second button (on its own art) is ONLINE and opens
//! the online login screen; without it that button keeps its stock label
//! and does nothing. Other Multiplayer and the gateway line are drawn but
//! do nothing; Display Options opens the same Options pages the game's Esc
//! menu shows, on the Video page.

use crate::ui::front_end::control::{vk, Action, Control, ControlKind};
use crate::ui::front_end::flow::Trigger;
use crate::ui::front_end::screen::{FrontCtx, Screen};
use crate::ui::front_end::{Registry, MAIN_MENU};

/// Background descriptor 8's cel (global, loaded by the preload):
/// `gameselectscreenEXP` when the expansion is installed, else `TitleScreen`.
pub const BACKGROUND: &str = r"FrontEnd\gameselectscreenEXP";
pub const BACKGROUND_CLASSIC: &str = r"FrontEnd\TitleScreen";
/// Button cels (all exist in the install).
pub const WIDE: &str = r"FrontEnd\WideButtonBlank";
pub const NARROW: &str = r"FrontEnd\NarrowButtonBlank";
pub const SHORT: &str = r"CharSelect\ShortButtonBlank";
/// The online button's art (descriptor 17, global
/// `d2exp.mpq`).
pub const WIDE2: &str = r"FrontEnd\WideButtonBlank02";
/// Logo halves: the black base cel (the fire overlay is drawn
/// additively over it by the host).
pub const LOGO_LEFT: &str = r"FrontEnd\D2logoBlackLeft";
pub const LOGO_RIGHT: &str = r"FrontEnd\D2logoBlackRight";
/// The game data version shown by owner request (the one allowed 1.14d
/// text): the install the D2RUST build runs.
pub const GAME_DATA_VERSION: &str = "v 1.14d";
/// The version line's box: frame x, bottom edge, width and height
/// (unchanged, so the D2RUST line sits where the version line did).
pub const VERSION_X: i32 = 0;
pub const VERSION_Y: i32 = 599;
pub const VERSION_W: u16 = 200;
pub const VERSION_H: u16 = 40;
/// The game data line's own box, directly below the D2RUST line.
pub const GAME_DATA_VERSION_Y: i32 = 599;
pub const GAME_DATA_VERSION_H: u16 = 20;
/// Font16 (descriptor 0x115, record).
pub const VERSION_FONT: u16 = 1;
/// Font8 (id 0 of `ui/text-fonts.tsv`): the front end's small text font
/// (the character screen's compact labels); its glyph cels are shorter
/// than Font16's (14 against 16 px).
pub const GAME_DATA_VERSION_FONT: u16 = 0;

/// The main menu's top version line: the D2RUST build version.
pub fn version_text() -> String {
    format!("D2RUST {}", crate::launch::VERSION)
}

/// Button string ids.
pub const STR_SINGLE_PLAYER: u32 = 5106;
pub const STR_BATTLE_NET: u32 = 5107;
/// The online button's label (D2RUST's own; with the `online` feature it
/// replaces string 5107).
pub const ONLINE_LABEL: &str = "ONLINE";
pub const STR_OTHER_MULTIPLAYER: u32 = 5108;
pub const STR_EXIT: u32 = 5109;
pub const STR_CREDITS: u32 = 5110;
/// The Display Options button: D2RUST's own, a wide button like Single
/// Player and Exit Game. Its label is written in capitals as the other
/// buttons' labels are. It sits centred in the gap between Other
/// Multiplayer (bottom edge 433) and Credits (top edge 503): 18 px above
/// it and 17 below.
pub const OPTIONS_LABEL: &str = "DISPLAY OPTIONS";
pub const OPTIONS_Y: i32 = 486;
pub const OPTIONS_H: u16 = 35;
/// "GATEWAY: %s" (string 0x2B29).
pub const STR_GATEWAY: u32 = 11049;

/// The default gateway list (`DATA\GLOBAL\gateways.txt`, version 1002):
/// zone (hours from GMT, as the file writes it) and name, in list order.
const GATEWAYS: [(i32, &str); 4] = [
    (8, "U.S. West"),
    (6, "U.S. East"),
    (-9, "Asia"),
    (-1, "Europe"),
];

/// The gateway picked when no choice is stored (`BNGatewayAccess::Load` →
/// `PickClosestZone`): the first zone whose distance to the system time-zone
/// bias (minutes) is smallest. The bias is 0 (UTC), as the main menu
/// check compares; the host's time zone is not read.
pub fn gateway_name() -> &'static str {
    let bias = 0;
    let mut best = (0x564, GATEWAYS[0].1);
    for (zone, name) in GATEWAYS {
        let d = (zone * 60 - bias).abs();
        if d < best.0 {
            best = (d, name);
        }
    }
    best.1
}

struct MainMenu;

fn button(
    art: &'static str,
    x: i32,
    y: i32,
    w: u16,
    h: u16,
    string: u32,
    action: Action,
) -> Control {
    // The gateway line and Other Multiplayer (and the second button
    // without the `online` feature) have no action but stay enabled: the
    // screen draws them as enabled buttons (a disabled one would draw with
    // mode 1). A click on them does nothing.
    Control::new(ControlKind::Button, x, y, w, h)
        .with_art(art)
        .with_string(string)
        .with_action(action)
}

/// Frame to draw for a resting, pressed or disabled button (r3 art): the same
/// rule the host applies. // D2RUST's own, unverified: disabled buttons draw
/// the up frames (the cels have no third state).
pub fn button_frame(c: &Control, tile: u32, pressed: bool) -> u32 {
    let tiles = crate::ui::front_end::control::button_tiles(c.w, c.h);
    crate::ui::front_end::control::button_frame(tile, tiles, pressed, c.enabled, false)
}

/// The second button: ONLINE with the `online` feature (on in shipped
/// builds), else the stock label with no action.
fn online_button(y: i32) -> Control {
    if cfg!(feature = "online") {
        button(WIDE2, 264, y, 272, 35, 0, Action::Trigger(Trigger::Online))
            .with_text(ONLINE_LABEL, 0)
    } else {
        button(WIDE2, 264, y, 272, 35, STR_BATTLE_NET, Action::None)
    }
}

impl Screen for MainMenu {
    fn build(&mut self, ctx: &mut FrontCtx) -> Vec<Control> {
        // Classic: the first four buttons sit 100 px higher.
        let up = if ctx.expansion { 0 } else { 100 };
        let bg = if ctx.expansion {
            BACKGROUND
        } else {
            BACKGROUND_CLASSIC
        };
        let mut controls = vec![
            Control::new(ControlKind::Image, 0, 599, 800, 600).with_art(bg),
            Control::new(ControlKind::AnimImage, 400, 120, 181, 170).with_art(LOGO_LEFT),
            Control::new(ControlKind::AnimImage, 400, 120, 188, 177).with_art(LOGO_RIGHT),
            button(
                WIDE,
                264,
                324 - up,
                272,
                35,
                STR_SINGLE_PLAYER,
                Action::Trigger(Trigger::SinglePlayer),
            ),
            online_button(366 - up),
            // The gateway label is set at run time: "GATEWAY: %s"
            // (string 11049) with the current gateway's name.
            button(NARROW, 264, 391 - up, 272, 25, STR_GATEWAY, Action::None)
                .with_text(gateway_name(), 0),
            button(
                WIDE,
                264,
                433 - up,
                272,
                35,
                STR_OTHER_MULTIPLAYER,
                Action::None,
            ),
            button(
                SHORT,
                332,
                528,
                135,
                25,
                STR_CREDITS,
                Action::Trigger(Trigger::Credits),
            ),
            button(
                WIDE,
                264,
                568,
                272,
                35,
                STR_EXIT,
                Action::Trigger(Trigger::Exit),
            )
            .with_hotkey(vk::ESC),
            {
                // r4: (0, 599) 200 × 40, flags 2 (centred), colour 0. The
                // D2RUST build version keeps the line's font and size.
                Control::new(
                    ControlKind::Text,
                    VERSION_X,
                    VERSION_Y,
                    VERSION_W,
                    VERSION_H,
                )
                .with_font(VERSION_FONT, 2)
                .with_text(version_text(), 0)
            },
            // The game data version, directly below the D2RUST line and in
            // the smaller font (its own text control: rows of one control
            // share a font).
            Control::new(
                ControlKind::Text,
                VERSION_X,
                GAME_DATA_VERSION_Y,
                VERSION_W,
                GAME_DATA_VERSION_H,
            )
            .with_font(GAME_DATA_VERSION_FONT, 2)
            .with_text(GAME_DATA_VERSION, 0),
        ];
        // Expansion creation order: descriptors 0x12,
        // 0x11, 0x13, 0x10 = gateway, online, Other Multiplayer, Single
        // Player.
        if ctx.expansion {
            let sp = controls.remove(3);
            controls.insert(6, sp);
            controls.swap(3, 4);
        }
        // D2RUST's own Display Options button, created after the other
        // buttons and before the two version lines (which stay the last
        // draws). It opens the Options pages the game's Esc menu shows, on
        // the Video page.
        let version = controls.len() - 2;
        controls.insert(
            version,
            button(
                WIDE,
                264,
                OPTIONS_Y,
                272,
                OPTIONS_H,
                0,
                Action::Trigger(Trigger::Options),
            )
            .with_text(OPTIONS_LABEL, 0),
        );
        controls
    }
}

pub fn register(reg: &mut Registry) {
    reg.register(MAIN_MENU, Box::new(MainMenu));
}
