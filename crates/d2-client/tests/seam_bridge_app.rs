//! Contract checks of the bridge ↔ Bevy app seam: both sides handle the
//! same value and must agree. No game files.

use bevy::prelude::*;
use d2_client::app::sound::{add_audio, AudioParts};
use d2_client::audio::driver::SoundRequest;
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::mirror::DynLink;
use d2_client::bridge::output::Output;
use d2_client::bridge::world::MONSTER;
use d2_client::bridge::{Bridge, BridgePlugin, BridgeResource, UnitKey};
use d2_client::world_view::present::deliver;
use d2_client::world_view::UiSounds;
use d2_proto::PROTOCOL_VERSION;

/// A link that ticks on every pump and delivers nothing.
struct Ticking;

impl ServerLink for Ticking {
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
        Ok(Pumped { ticked: true })
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        Vec::new()
    }
}

#[test]
fn audio_outputs_become_sound_requests_in_list_order_with_the_captured_position() {
    let mut bridge = Bridge::new(Ticking).unwrap();
    let unit = UnitKey::new(MONSTER, 7);
    let list = [
        Output::ServerSound {
            unit,
            class: 5,
            at: Some((0x1241, 0x11C4)),
            event: 3,
        },
        Output::UnitFreed {
            unit,
            client_only: false,
        },
    ];
    let requests = deliver(&mut bridge, &list, None).unwrap();
    assert_eq!(
        requests,
        vec![
            SoundRequest::Server {
                unit,
                class: 5,
                at: Some((0x1241, 0x11C4)),
                event: 3,
            },
            SoundRequest::UnitFreed {
                unit,
                client_only: false,
            },
        ]
    );
}

#[derive(Resource, Default)]
struct Pushed(u32);

/// A UI system of the frame (the world view's UI pass runs in `Update`)
/// asking for a click sound.
fn ui_click_sound(mut sounds: ResMut<UiSounds>, mut pushed: ResMut<Pushed>) {
    sounds.0.push(SoundRequest::Ui(1));
    pushed.0 += 1;
}

#[test]
fn a_ui_sound_asked_for_in_update_is_taken_by_the_same_frames_audio_frame() {
    let bridge = Bridge::new(Box::new(Ticking) as DynLink).unwrap();
    let mut app = App::new();
    // A fixed host clock: no wall time.
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(
        std::sync::Arc::new(std::sync::atomic::AtomicU32::new(1000)),
    ));
    app.add_plugins(MinimalPlugins)
        .add_plugins(BridgePlugin)
        .insert_resource(BridgeResource(bridge))
        .init_resource::<UiSounds>()
        .init_resource::<Pushed>();
    add_audio(&mut app, AudioParts::empty());
    app.add_systems(Update, ui_click_sound);
    for frame in 1..=5 {
        app.update();
        assert_eq!(app.world().resource::<Pushed>().0, frame);
        // Drained in the frame it was asked for, never left for the next
        // frame's audio (a later server tick).
        assert!(
            app.world().resource::<UiSounds>().0.is_empty(),
            "frame {frame}: the UI sound waits for the next audio frame"
        );
    }
    // The order above must not be a scheduler accident: the audio frame
    // runs after every `Update` system, so `add_audio` adds nothing to
    // `Update` and one system to `PostUpdate`.
    let counts = |audio: bool| {
        let bridge = Bridge::new(Box::new(Ticking) as DynLink).unwrap();
        let mut app = App::new();
        // A fixed host clock: no wall time.
        app.insert_resource(d2_client::bridge::mirror::ScriptedClock(
            std::sync::Arc::new(std::sync::atomic::AtomicU32::new(1000)),
        ));
        app.add_plugins(MinimalPlugins)
            .add_plugins(BridgePlugin)
            .insert_resource(BridgeResource(bridge))
            .init_resource::<UiSounds>();
        if audio {
            add_audio(&mut app, AudioParts::empty());
        }
        let schedules = app.world().resource::<Schedules>();
        let len = |s: Option<&Schedule>| s.map_or(0, Schedule::systems_len);
        (len(schedules.get(Update)), len(schedules.get(PostUpdate)))
    };
    let (without, with) = (counts(false), counts(true));
    assert_eq!(with.0, without.0, "add_audio put a system in Update");
    assert_eq!(
        with.1,
        without.1 + 1,
        "the audio frame is not in PostUpdate"
    );
}

#[test]
fn the_presented_image_sits_where_the_click_mapping_reads_it() {
    use d2_client::ui::Presentation;
    for (w, h) in [
        (1601u32, 1201u32),
        (1600, 1200),
        (2401, 1300),
        (801, 601),
        (1700, 1301),
    ] {
        let p = Presentation::new(w, h).unwrap();
        let s = p.scale as f32;
        // A sprite centred on the window, moved by the offset (y up): its
        // top-left in physical window pixels (y down).
        let (dx, dy) = p.centre_offset();
        let left = w as f32 / 2.0 + dx - 400.0 * s;
        let top = h as f32 / 2.0 - dy - 300.0 * s;
        let (fx, fy) = p.from_frame(0, 0);
        assert_eq!((left, top), (fx as f32, fy as f32), "window {w} × {h}");
    }
}
