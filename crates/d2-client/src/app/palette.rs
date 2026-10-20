//! The act palette of the presented frame: the act
//! `a`'s `DATA\GLOBAL\palette\act<n>\pal.pl2` with `n = a + 1` (`n`
//! outside 1…5 → 1), loaded at game start for act 0 and switched when the
//! model's palette act changes (S→C 0x03; a room
//! change to a level whose Levels `Pal` (+0x02, not `Act`) differs,
//! which hands `Pal` as `a`). The five files are read from the
//! user's archives once, up front; a missing file is an error.

use crate::assets::path::{read_pl2, FileSource};
use bevy::prelude::*;

use crate::bridge::mirror::bridge_frame;
use crate::bridge::BridgeResource;
use crate::scene::present_palette;
use crate::world_view::WorldViewState;

/// The archive path of act `a`'s palette.
pub fn act_palette_path(a: u8) -> String {
    let n = match u32::from(a) + 1 {
        n @ 1..=5 => n,
        _ => 1,
    };
    format!("DATA\\GLOBAL\\palette\\act{n}\\pal.pl2")
}

/// The `pal.pl2` of each act and the act whose palette is presented.
#[derive(Resource, Debug, Clone)]
pub struct ActPalettes {
    /// Index `a` = act `a` (0…4).
    pub pl2: [Vec<u8>; 5],
    /// The act whose palette the world view presents; `None` until the
    /// first switch.
    pub shown: Option<u8>,
}

impl ActPalettes {
    /// Reads the five act palettes from the archives.
    pub fn live(archives: &dyn FileSource) -> Result<Self, String> {
        // The flat file layout back from the decoded table (`Pl2::to_bytes`):
        // one path for the archives and a native folder.
        let read = |a: u8| {
            let path = act_palette_path(a);
            read_pl2(archives, &path)
                .ok_or_else(|| format!("{path}: in no archive"))?
                .map(|p| p.to_bytes())
                .map_err(|e| format!("{path}: {e}"))
        };
        Ok(ActPalettes {
            pl2: [read(0)?, read(1)?, read(2)?, read(3)?, read(4)?],
            shown: None,
        })
    }

    /// The act to present for the model's palette act: the client loop
    /// loads act 0's palette at game start, so act 0 until
    /// 0x03 sets one.
    pub fn wanted(palette_act: Option<u8>) -> u8 {
        palette_act.unwrap_or(0)
    }

    /// The `pal.pl2` of act `a` (`n` outside 1…5 → act 1's).
    pub fn of(&self, a: u8) -> &[u8] {
        let i = if a < 5 { usize::from(a) } else { 0 };
        &self.pl2[i]
    }
}

/// Switches the world view's presented palette to the model's palette
/// act after each bridge frame. A palette file that is not a valid
/// `pal.pl2` is an error.
pub fn present_act_palette(
    bridge: Res<BridgeResource>,
    mut palettes: ResMut<ActPalettes>,
    mut state: ResMut<WorldViewState>,
) -> Result {
    let act = ActPalettes::wanted(bridge.0.world().palette_act);
    if palettes.shown == Some(act) {
        return Ok(());
    }
    state.assets.palette = present_palette(palettes.of(act))?;
    palettes.shown = Some(act);
    Ok(())
}

/// The systems of the wiring below, once per app (`app::session`).
pub fn register_act_palettes(app: &mut App) {
    app.add_systems(
        PreUpdate,
        present_act_palette
            .after(bridge_frame)
            .run_if(resource_exists::<BridgeResource>)
            .run_if(resource_exists::<WorldViewState>)
            .run_if(resource_exists::<ActPalettes>),
    );
}

/// Adds the act palettes and their switch (after the bridge frame, so the
/// frame built in `Update` presents the palette of the model it draws).
pub fn add_act_palettes(app: &mut impl super::session::Setup, palettes: ActPalettes) {
    app.register("act-palettes", register_act_palettes);
    app.world_mut().insert_resource(palettes);
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use d2_formats::palette::Rgb;
    use d2_proto::PROTOCOL_VERSION;

    use super::*;
    use crate::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
    use crate::bridge::{Bridge, BridgePlugin};
    use crate::world_view::{ModelFeed, NoFeed, Unspecified, ViewAssets};

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

    /// A `pal.pl2` whose index 1 is (a, a, a).
    fn pl2(a: u8) -> Vec<u8> {
        let mut b = vec![0; 1024];
        b[4..7].copy_from_slice(&[a, a, a]);
        b
    }

    #[test]
    fn the_presented_palette_follows_the_palette_act() {
        // Frame 1: nothing received (act 0 at game start); frame 2: 0x03
        // for act 1 (`03 01 …`).
        let load_act_2 = vec![vec![
            0x03, 0x01, 0xC4, 0x88, 0x38, 0x10, 0x28, 0x00, 0x61, 0xD1, 0xE0, 0x9F,
        ]];
        let link = Script(VecDeque::from([vec![], load_act_2]), vec![]);
        let bridge = Bridge::new(Box::new(link) as _).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(BridgePlugin)
            .insert_resource(BridgeResource(bridge))
            .insert_resource(WorldViewState::new(
                ViewAssets::new(crate::app::play::unspecified_palette()),
                Box::new(Unspecified),
                Box::new(ModelFeed::<NoFeed>::default()),
            ));
        add_act_palettes(
            &mut app,
            ActPalettes {
                pl2: [pl2(10), pl2(20), pl2(30), pl2(40), pl2(50)],
                shown: None,
            },
        );
        let index1 = |app: &App| {
            app.world()
                .resource::<WorldViewState>()
                .assets
                .palette
                .colors[1]
        };
        app.update();
        assert_eq!(
            index1(&app),
            Rgb {
                r: 10,
                g: 10,
                b: 10
            }
        );
        app.update();
        assert_eq!(
            app.world()
                .resource::<BridgeResource>()
                .0
                .world()
                .palette_act,
            Some(1)
        );
        assert_eq!(
            index1(&app),
            Rgb {
                r: 20,
                g: 20,
                b: 20
            }
        );
        assert_eq!(app.world().resource::<ActPalettes>().shown, Some(1));
    }

    #[test]
    fn act_palette_paths() {
        assert_eq!(act_palette_path(0), "DATA\\GLOBAL\\palette\\act1\\pal.pl2");
        assert_eq!(act_palette_path(4), "DATA\\GLOBAL\\palette\\act5\\pal.pl2");
        // n outside 1…5 → 1.
        assert_eq!(act_palette_path(5), "DATA\\GLOBAL\\palette\\act1\\pal.pl2");
        assert_eq!(
            act_palette_path(255),
            "DATA\\GLOBAL\\palette\\act1\\pal.pl2"
        );
        assert_eq!(ActPalettes::wanted(None), 0);
        assert_eq!(ActPalettes::wanted(Some(2)), 2);
    }
}
