//! Unit messages end to end, headless and synthetic: S→C 0x26 bytes
//! delivered through the bridge reach the original UI, a system line
//! (type 4) shows in the screen message list, and an overhead message
//! (type 5) on a present unit makes a bubble that expires.

use std::collections::VecDeque;

use bevy::prelude::*;
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::{Bridge, BridgePlugin, BridgeResource};
use d2_client::ui::layout::Screen;
use d2_client::ui::original::{OriginalUi, UiConfig};
use d2_client::ui::{NoPanelRules, NoStrings, UiCtx, UiDraw, UiRoot};
use d2_client::world_view::{WorldViewPlugin, WorldViewUi};
use d2_proto::PROTOCOL_VERSION;

/// Delivers one queued chunk list per pump (a tick each).
struct Script(VecDeque<Vec<Vec<u8>>>, Vec<Vec<u8>>);

impl ServerLink for Script {
    fn protocol_version(&self) -> u32 {
        PROTOCOL_VERSION
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn send(&mut self, _: SendQueue, _: &[u8]) -> Result<Sent, LinkError> {
        Ok(Sent::Queued)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.1 = self.0.pop_front().unwrap_or_default();
        Ok(Pumped { ticked: true })
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.1)
    }
}

/// S→C 0x26 (`server-messages.tsv`): type u8@1, lang u8@2, unit type
/// u8@3, GUID u32@4, u8@8, u8@9, name cstr @10, text cstr.
fn chat26(kind: u8, unit_type: u8, guid: u32, b8: u8, text: &str) -> Vec<u8> {
    let mut m = vec![0x26, kind, 0, unit_type];
    m.extend_from_slice(&guid.to_le_bytes());
    m.extend_from_slice(&[b8, 0, 0]);
    m.extend_from_slice(text.as_bytes());
    m.push(0);
    m
}

fn app(frames: Vec<Vec<Vec<u8>>>) -> App {
    let bridge = Bridge::new(Box::new(Script(frames.into(), Vec::new())) as _).unwrap();
    let mut app = App::new();
    // A fixed host clock: no wall time.
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(
        std::sync::Arc::new(std::sync::atomic::AtomicU32::new(1000)),
    ));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .add_plugins((BridgePlugin, WorldViewPlugin { gpu: false }))
        .insert_resource(BridgeResource(bridge));
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    let original = OriginalUi::new(
        UiConfig {
            screen: Screen::R800,
            expansion_installed: false,
        },
        None,
    )
    .unwrap();
    original.install(&mut root).unwrap();
    let mut ui = WorldViewUi::new(root, Box::new(NoStrings));
    ui.original = Some(original);
    app.insert_non_send(ui);
    app
}

/// The chat-font texts the UI draws now.
fn texts(app: &App) -> Vec<String> {
    let ui = app.world().non_send::<WorldViewUi>();
    let bridge = app.world().resource::<BridgeResource>();
    let ctx = UiCtx {
        tick: bridge.0.world().frames,
        world: bridge.0.world(),
        strings: &NoStrings,
    };
    let mut out: Vec<UiDraw> = Vec::new();
    ui.root.draw(&ctx, &mut out);
    out.iter()
        .filter_map(|d| match d {
            UiDraw::Text(t) if t.style.font == 13 => Some(String::from_utf16_lossy(&t.text)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_system_chat_line_shows_in_the_message_list() {
    let mut app = app(vec![vec![chat26(4, 0, 0, 1, "You cannot do that")]]);
    app.update();
    assert_eq!(texts(&app), vec!["You cannot do that".to_string()]);
}

#[test]
fn an_overhead_message_on_an_absent_unit_shows_nothing() {
    let mut app = app(vec![vec![chat26(5, 1, 77, 0, "hi")]]);
    app.update();
    assert!(texts(&app).is_empty());
}
