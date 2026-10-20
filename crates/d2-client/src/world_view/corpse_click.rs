//! The click on a player's corpse in the play preview: a left press on a
//! drawn mode-17 player unit (the local player's corpse)
//! sends C→S 0x16 PickItem with unit type 0 (:
//! a dead player in range is the corpse take-back);
//! the server decides everything (owner test, what moves, the corpse's
//! removal). step 7: this module only asks.
//!
//! D2RUST's own, unverified (the play preview): the base game hovers the unit
//! (no hover model) and interacts with it;
//! the preview picks the corpse whose feet are nearest the
//! press ([`crate::bridge::hover`] box), asks at once and, like the
//! ground items, walks to it and asks again when the walk has ended (the
//! server's walk toward a unit is a seam).

use crate::bridge::hover::{unit_feet, HIT_ABOVE, HIT_BELOW, HIT_HALF_WIDTH};
use crate::bridge::link::ServerLink;
use crate::bridge::modes::player_mode;
use crate::bridge::world::{ClientWorld, UnitKey};
use crate::bridge::{items, Bridge, BridgeError};
use crate::rules::camera::{Camera, FrameSize};
use crate::ui::{PointerButton, UiEvent};

use super::ground_items::{PICK_RETRIES, PICK_WAIT_FRAMES};

/// Unit type of a player.
const PLAYER: u8 = 0;

/// C→S 0x16 PickItem on a corpse: unit type 0, cursor flag 0.
pub fn pick_corpse(guid: u32) -> d2_proto::client::PickItem {
    d2_proto::client::PickItem {
        type_: u32::from(PLAYER),
        id: guid,
        cursor: 0,
    }
}

/// The corpses of the world: player-type units in mode 17 other than the
/// local player, with their subtile cell.
pub fn corpses(w: &ClientWorld) -> Vec<(UnitKey, (u16, u16))> {
    w.units
        .iter()
        .filter(|(k, u)| {
            k.unit_type == PLAYER && Some(**k) != w.local_player && u.mode == player_mode::DEAD
        })
        .filter_map(|(k, u)| Some((*k, u.position?)))
        .collect()
}

/// The click waiting for its walk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Pending {
    guid: u32,
    walked: bool,
    seen_walk: bool,
    frames: u32,
    retries: u32,
}

/// The corpse clicks of the world view.
#[derive(Debug, Default)]
pub struct CorpseClicks {
    pending: Option<Pending>,
}

impl CorpseClicks {
    /// The corpse under screen point `at` for `camera`, nearest feet first.
    pub fn hit(w: &ClientWorld, camera: &Camera, at: (i32, i32)) -> Option<UnitKey> {
        let mut best: Option<(i32, UnitKey)> = None;
        for (key, cell) in corpses(w) {
            let (fx, fy) = unit_feet(camera, PLAYER, cell);
            let (dx, dy) = (at.0 - fx, at.1 - fy);
            if dx.abs() > HIT_HALF_WIDTH || !(-HIT_ABOVE..=HIT_BELOW).contains(&dy) {
                continue;
            }
            let d = dx.abs() + dy.abs();
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, key));
            }
        }
        best.map(|(_, k)| k)
    }

    /// The left presses on a corpse (with no item on the cursor): C→S 0x16
    /// type 0, taken out of the returned list; the rest go on.
    pub fn take_clicks<L: ServerLink>(
        &mut self,
        bridge: &mut Bridge<L>,
        camera: Option<&Camera>,
        unhandled: &[UiEvent],
    ) -> Result<Vec<UiEvent>, BridgeError> {
        let mut rest = Vec::with_capacity(unhandled.len());
        for e in unhandled {
            if let (
                UiEvent::Press {
                    button: PointerButton::Left,
                    at,
                },
                Some(cam),
                None,
            ) = (*e, camera, items::cursor_item(bridge.world()))
            {
                if let Some(k) = Self::hit(bridge.world(), cam, (at.x, at.y)) {
                    bridge.send(&pick_corpse(k.guid))?;
                    self.pending = Some(Pending {
                        guid: k.guid,
                        ..Pending::default()
                    });
                    continue;
                }
            }
            rest.push(*e);
        }
        Ok(rest)
    }

    /// One frame of the click waiting for its walk (as
    /// `GroundItems::frame`): walk to the corpse, then ask again when the
    /// walk has ended. The record ends when the corpse is gone.
    pub fn frame<L: ServerLink>(
        &mut self,
        bridge: &mut Bridge<L>,
        walking: bool,
    ) -> Result<(), BridgeError> {
        let Some(mut p) = self.pending else {
            return Ok(());
        };
        let Some((_, (x, y))) = corpses(bridge.world())
            .into_iter()
            .find(|(k, _)| k.guid == p.guid)
        else {
            self.pending = None;
            return Ok(());
        };
        p.frames += 1;
        if !p.walked {
            p.walked = true;
            p.frames = 0;
            bridge.send(&d2_proto::client::Walk { x, y })?;
        } else if walking {
            p.seen_walk = true;
            p.frames = 0;
        } else if p.frames >= PICK_WAIT_FRAMES || p.seen_walk {
            if p.retries >= PICK_RETRIES {
                self.pending = None;
                return Ok(());
            }
            p.retries += 1;
            p.seen_walk = false;
            p.frames = 0;
            bridge.send(&pick_corpse(p.guid))?;
        }
        self.pending = Some(p);
        Ok(())
    }
}

/// The camera of the local player for the clicks (no
/// shake), as the world clicks build it: at the local player's one
/// position of the frame ([`ClientWorld::local_position`]).
pub fn camera_for(w: &ClientWorld, open_mode: u8) -> Option<Camera> {
    use crate::rules::camera::{moving_to_client, OpenMode};
    let (x16, y16) = w.local_position()?;
    let at = moving_to_client(x16, y16);
    let mode = OpenMode::new(open_mode).unwrap_or(OpenMode::NONE);
    Some(Camera::new(FrameSize::play(), mode, at, (0, 0)))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use d2_proto::PROTOCOL_VERSION;

    use super::*;
    use crate::bridge::dispatch::Dispatch;
    use crate::bridge::link::{LinkError, Pumped, SendQueue, Sent};
    use crate::bridge::world::{ClientUnit, KindData, PlayerData};
    use crate::ui::Point;

    #[derive(Clone, Default)]
    struct RecordingLink {
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl ServerLink for RecordingLink {
        fn protocol_version(&self) -> u32 {
            PROTOCOL_VERSION
        }
        fn tick_rate(&self) -> d2_sim::tick::TickRate {
            d2_sim::tick::TickRate::BASE
        }
        fn send(&mut self, _: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
            self.sent.lock().unwrap().push(msg.to_vec());
            Ok(Sent::Queued)
        }
        fn pump(&mut self) -> Result<Pumped, LinkError> {
            Ok(Pumped { ticked: false })
        }
        fn receive(&mut self) -> Vec<Vec<u8>> {
            Vec::new()
        }
    }

    const ME: UnitKey = UnitKey {
        unit_type: 0,
        guid: 1,
    };
    const CORPSE: UnitKey = UnitKey {
        unit_type: 0,
        guid: 9,
    };

    fn press(at: (i32, i32)) -> UiEvent {
        UiEvent::Press {
            button: PointerButton::Left,
            at: Point::new(at.0, at.1),
        }
    }

    /// The local player at (100, 100); a unit in `mode` at (102, 100).
    fn scene(mode: u32) -> (Bridge<RecordingLink>, RecordingLink) {
        let link = RecordingLink::default();
        let mut b = Bridge::with_dispatch(link.clone(), Dispatch::empty()).unwrap();
        let w = b.world_mut();
        let mut p = ClientUnit::new(ME);
        p.position = Some((100, 100));
        p.kind = KindData::Player(PlayerData::default());
        w.units.insert(ME, p);
        w.local_player = Some(ME);
        let mut c = ClientUnit::new(CORPSE);
        c.position = Some((102, 100));
        c.mode = mode;
        w.units.insert(CORPSE, c);
        (b, link)
    }

    #[test]
    fn a_press_on_a_corpse_sends_pick_item_type_0_then_walks_and_asks_again() {
        let (mut b, link) = scene(player_mode::DEAD);
        let cam = camera_for(b.world(), 0).unwrap();
        let at = unit_feet(&cam, PLAYER, (102, 100));
        let mut c = CorpseClicks::default();
        let rest = c
            .take_clicks(&mut b, Some(&cam), &[press(at), press((5, 5))])
            .unwrap();
        assert_eq!(rest, [press((5, 5))], "the empty-ground press goes on");
        // C→S 0x16: type 0 (a player), the corpse GUID, cursor flag 0.
        let pick = vec![0x16, 0, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            link.sent.lock().unwrap().as_slice(),
            std::slice::from_ref(&pick)
        );
        // The next frame walks to it; when the walk has ended, asks again.
        c.frame(&mut b, false).unwrap();
        assert_eq!(link.sent.lock().unwrap().len(), 2, "the walk");
        c.frame(&mut b, true).unwrap();
        c.frame(&mut b, false).unwrap();
        assert_eq!(link.sent.lock().unwrap().last(), Some(&pick));
        // The corpse gone: the record ends.
        b.world_mut().units.remove(&CORPSE);
        c.frame(&mut b, false).unwrap();
        assert_eq!(link.sent.lock().unwrap().len(), 3);
    }

    #[test]
    fn only_a_dead_player_unit_is_a_corpse() {
        let (mut b, link) = scene(player_mode::TOWN_NEUTRAL);
        let cam = camera_for(b.world(), 0).unwrap();
        let at = unit_feet(&cam, PLAYER, (102, 100));
        let mut c = CorpseClicks::default();
        let rest = c.take_clicks(&mut b, Some(&cam), &[press(at)]).unwrap();
        assert_eq!(rest.len(), 1);
        assert!(link.sent.lock().unwrap().is_empty());
        // The local player's own dying body is not a corpse either.
        b.world_mut().units.get_mut(&ME).unwrap().mode = player_mode::DEAD;
        assert!(corpses(b.world()).is_empty());
    }
}
