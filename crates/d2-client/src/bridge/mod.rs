//! The only link between the Bevy app and the game. Outbound: requests
//! become C→S bytes built with `d2-proto` ([`intent`]). Inbound: the
//! S→C bytes the server delivers are split with `d2-proto` and dispatched
//! by id to handlers that update the [`world::ClientWorld`] ([`receive`],
//! [`dispatch`]); ids no handler owns yet are counted, never interpreted.
//! Unit-handler messages are queued on their unit and applied by the
//! update pass ([`update`]) in frames where the server ticked; the C→S
//! messages the model answers with on its own (0x6B, 0x5F) go through the
//! send path at the end of the frame.
//! On a mod game's wire, its mod messages pass both ways in envelopes:
//! [`Bridge::send_mod`] out, a [`ModMessages`] hook in.
//! Bevy entities only mirror the world model ([`mirror`]). Contains no
//! game rules: the server decides every outcome.
//!
//! Everything except [`mirror`] is plain Rust without Bevy types.

pub mod attack_weapon;
pub mod belt;
pub mod bits;
pub mod blood_spray;
pub mod chat_end;
pub mod check;
pub mod click;
pub mod client_do;
mod client_do_mon;
pub mod client_missiles;
pub mod client_path;
pub mod combat;
pub mod critter_path;
pub mod critters;
pub mod disguise;
#[cfg(test)]
mod disguise_tests;
pub mod dispatch;
pub mod drlg;
pub mod hit_overlays;
pub mod hover;
pub mod inject;
pub mod intent;
pub mod item_lists;
pub mod items;
pub mod link;
pub mod local;
pub mod mirror;
pub mod modes;
#[cfg(test)]
mod modes_tests;
pub mod monster_anim;
pub mod motion;
pub mod msg;
pub mod objects;
pub mod output;
pub mod overlay_lights;
pub mod passive;
#[cfg(test)]
mod passive_tests;
pub mod player_anim;
pub mod poke;
pub mod predict;
pub mod receive;
#[cfg(feature = "online")]
pub mod remote;
#[cfg(feature = "rng-trace")]
pub mod rng_trace;
pub mod skill_fallback;
pub mod skills;
pub mod state;
pub mod state_overlays;
pub mod update;
pub mod use_state;
pub mod world;

#[cfg(test)]
mod gaps_numbered_tests;
#[cfg(test)]
mod local_tests;
#[cfg(test)]
mod mod_messages_tests;
#[cfg(test)]
mod seam_movement_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_c2cli;

use crate::rules::lighting::records::LightList;
use d2_proto::envelope::Wire;
use d2_proto::transport::SplitError;
use d2_proto::{FixedMessage, PROTOCOL_VERSION};

use dispatch::{Dispatch, TableError};
use intent::IntentError;
use link::{LinkError, Sent, ServerLink};
use output::Output;
pub use receive::ModMessages;
use receive::{receive_chunk_on, ModInbox, ReceiveLog};
use world::{ClientTables, ClientWorld, ModelInputs, VisibleFn};

pub use link::{Pumped, SendQueue, LOCAL_CLIENT};
pub use local::{LocalLink, SinglePlayer};
pub use mirror::{BridgePlugin, BridgeResource, FrameOutputs, UnitView};
pub use world::{ClientUnit, UnitKey};

/// A bridge failure. All of them stop the frame.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("server protocol version {server}, client {client}")]
    Version { client: u32, server: u32 },
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Intent(#[from] IntentError),
    #[error(transparent)]
    Link(#[from] LinkError),
    /// A chunk the split refuses: a message too large or running past
    /// the chunk's end.
    #[error("S→C chunk refused: {0}")]
    Split(#[from] SplitError),
    /// A world click that cannot be resolved (its handler refused it).
    #[error("world click: {0}")]
    Click(dispatch::HandlerError),
}

/// What one bridge frame did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameReport {
    pub ticked: bool,
    pub chunks: usize,
    pub messages: usize,
    pub handled: usize,
    /// Unit-handler messages queued on their unit.
    pub queued: usize,
    /// Unit-handler messages whose unit was not in the model.
    pub dropped: usize,
    /// Queued messages the update pass applied.
    pub drained: usize,
    pub unowned: usize,
    pub rejected: usize,
    pub discarded_bytes: usize,
    /// C→S messages the model sent on its own (0x6B, 0x5F).
    pub answered: usize,
    /// UI and sound outputs the frame's handlers made.
    pub outputs: usize,
    /// A paused frame: no pump, no receive.
    pub paused: bool,
}

/// The time between two client pings, 5 s: [`Bridge::ping`] waits this
/// long in server ticks at the game's rate (125 ticks at 25 Hz, 300 at
/// 60 Hz).
pub const PING_MS: u32 = 5_000;

/// C→S 0x6D as D2RUST sends it: the id, then the client's clock u32@1 and
/// last round trip u32@5 (zero: D2RUST keeps no wall clock) and four
/// zero bytes: 13 bytes.
pub const PING_MESSAGE: [u8; 13] = [0x6D, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// The bridge: a server link, the client world model and the dispatch
/// table.
pub struct Bridge<L> {
    link: L,
    dispatch: Dispatch,
    world: ClientWorld,
    inputs: ModelInputs,
    log: ReceiveLog,
    /// The frame's UI and sound outputs, in order.
    outputs: Vec<Output>,
    /// UI state 9 or 11 is open ([`Bridge::set_paused`]).
    paused: bool,
    /// The previous display pass was paused: resume without catching up its time.
    display_was_paused: bool,
    /// A control channel holds the game ([`Bridge::set_held`]).
    held: bool,
    /// The link delivered server monster paths ([`ServerLink::server_paths`]):
    /// the verified copy moves them, the preview motion stands down.
    server_copy: bool,
    /// C→S ids the model's own sends drop instead of sending (tool use:
    /// `state-dump --no-own-c2s`); empty in play.
    drop_own: Vec<u8>,
    /// The messages dropped so far, taken by [`Bridge::take_dropped`].
    dropped: Vec<Vec<u8>>,
    /// The local player's aim waits for its own client path
    /// ([`Bridge::set_local_aim_late`]); `held_aim` is the copy's read of
    /// it this tick, aimed by [`Bridge::aim_local`].
    local_aim_late: bool,
    held_aim: Option<critter_path::ServerPath>,
    /// The client ping: the server tick count
    /// at the last C→S 0x6D, none before the first; on only when the app
    /// runs a game ([`Bridge::set_ping`]).
    ping_frame: Option<u64>,
    ping_on: bool,
    /// The wire the bridge speaks and the mod messages received on it
    /// (MS13); D2RUST's own wire unless a mod game sets a hook.
    mods: ModInbox,
    /// A mod game's handler of its S→C mod messages
    /// ([`Bridge::set_mod_messages`]).
    mod_hook: Option<Box<dyn ModMessages>>,
}

impl<L: ServerLink> Bridge<L> {
    /// A bridge over `link` with D2RUST's dispatch table.
    pub fn new(link: L) -> Result<Self, BridgeError> {
        Self::with_dispatch(link, Dispatch::from_spec()?)
    }

    /// A bridge with a given dispatch table (tests and tools). Refuses a
    /// link of another protocol version.
    pub fn with_dispatch(link: L, dispatch: Dispatch) -> Result<Self, BridgeError> {
        let server = link.protocol_version();
        if server != PROTOCOL_VERSION {
            return Err(BridgeError::Version {
                client: PROTOCOL_VERSION,
                server,
            });
        }
        let world = ClientWorld {
            tick_rate: link.tick_rate(),
            ..ClientWorld::default()
        };
        Ok(Self {
            link,
            dispatch,
            world,
            inputs: ModelInputs::default(),
            log: ReceiveLog::default(),
            outputs: Vec::new(),
            paused: false,
            display_was_paused: false,
            held: false,
            server_copy: false,
            drop_own: Vec::new(),
            dropped: Vec::new(),
            local_aim_late: false,
            held_aim: None,
            ping_frame: None,
            ping_on: false,
            mods: ModInbox::default(),
            mod_hook: None,
        })
    }

    /// Clears the game when the server re-enters this client after a lost
    /// connection (contracts 3.7): the whole bridge world, the player's own
    /// character state included, goes; the server's resend rebuilds it. What
    /// stays is the client's own data, not game state: the tick rate, the
    /// item, monster and object tables, the animation handles, the client
    /// GUID counter and the frame/tick bookkeeping.
    fn clear_after_resume(&mut self) {
        let old = std::mem::take(&mut self.world);
        self.world = ClientWorld {
            tick_rate: old.tick_rate,
            item_tables: old.item_tables,
            class_vitals: old.class_vitals,
            stat_ops: old.stat_ops,
            tip_anims: old.tip_anims,
            corpse_tables: old.corpse_tables,
            gfx_ready: old.gfx_ready,
            session_total: old.session_total,
            session_log: old.session_log,
            frames: old.frames,
            server_ticks: old.server_ticks,
            objclient: objects::ClientObjects {
                selectable: old.objclient.selectable,
                hover_layers: old.objclient.hover_layers,
                monster_pieces: old.objclient.monster_pieces,
                next_guid: old.objclient.next_guid,
                ..objects::ClientObjects::default()
            },
            ..ClientWorld::default()
        };
        self.held_aim = None;
        self.server_copy = false;
        self.inputs.objclient.quest_flags = None;
        self.outputs.clear();
    }

    /// Sends a typed C→S message.
    pub fn send<M: FixedMessage>(&mut self, msg: &M) -> Result<Sent, BridgeError> {
        self.send_bytes(&intent::encode(msg))
    }

    /// Save and Exit Game: C→S 0x69 on the system queue (no duplicate
    /// filter), then `exit_requested` := 1. The client stays in the game
    /// until the server's 0x05.
    pub fn save_and_exit(&mut self) -> Result<Sent, BridgeError> {
        let sent = self.send_bytes(&[0x69])?;
        self.world.exit_requested = true;
        Ok(sent)
    }

    /// The client's own ping on or off (off by default: a bridge in a
    /// test sends only what the test asks for). The game's runs turn it
    /// on (`play`, `state-dump` unless `--no-ping`, a tool that sends
    /// its own pings).
    pub fn set_ping(&mut self, on: bool) {
        self.ping_on = on;
    }

    /// The client ping, from the loop's timer: C→S 0x6D at the first
    /// pass, then at the first pass [`PING_MS`] of server ticks past the
    /// last ping, before the pump, so the server reads it before the next
    /// tick and its 0x8F ends that tick's batch. The bytes after the id
    /// would be the client's clock and the last round trip; D2RUST keeps
    /// no wall clock here and sends zeros. The server's answer does not
    /// read them.
    fn ping(&mut self) -> Result<(), BridgeError> {
        let now = self.world.server_ticks;
        let period = u64::from(self.world.tick_rate.ticks_for_ms(PING_MS));
        // a tick count below the last ping (a rejoined game): due again
        if !self.ping_on
            || self
                .ping_frame
                .is_some_and(|f| now.checked_sub(f).is_some_and(|d| d < period))
        {
            return Ok(());
        }
        self.ping_frame = Some(now);
        self.send_bytes(&PING_MESSAGE)?;
        Ok(())
    }

    /// Sends C→S bytes after the classifier check on the bridge's wire.
    pub fn send_bytes(&mut self, msg: &[u8]) -> Result<Sent, BridgeError> {
        let queue = intent::route_on(self.mods.wire, msg)?;
        Ok(self.link.send(queue, msg)?)
    }

    /// Makes the bridge speak a mod game's wire (MS13): `hook` gets every
    /// S→C mod message, and [`Bridge::send_mod`] sends C→S ones. The host
    /// must speak the mod wire too (`Started::wire`).
    pub fn set_mod_messages(&mut self, hook: Box<dyn ModMessages>) {
        self.mods.wire = Wire::Mod;
        self.mod_hook = Some(hook);
    }

    /// The wire the bridge speaks: [`Wire::Mod`] once a mod hook is set.
    pub fn wire(&self) -> Wire {
        self.mods.wire
    }

    /// Sends a mod message: `payload` in a C→S envelope. Refused on
    /// D2RUST's own wire, like any unknown id.
    pub fn send_mod(&mut self, payload: &[u8]) -> Result<Sent, BridgeError> {
        let msg = intent::mod_message(payload)?;
        self.send_bytes(&msg)
    }

    /// Hands the mod messages received so far to the mod hook, in order.
    fn run_mod_hook(&mut self) {
        if self.mods.payloads.is_empty() {
            return;
        }
        let payloads = std::mem::take(&mut self.mods.payloads);
        if let Some(hook) = self.mod_hook.as_mut() {
            for p in &payloads {
                hook.message(&mut self.world, p);
            }
        }
    }

    /// Whether the single-player game is paused: UI state 9 (the Esc
    /// menu) or 11 is open. Set by the app before each frame.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Whether a control channel (`play --control`) holds the game: the
    /// same pause the Esc menu gives, kept apart from the menu's flag so
    /// either one pauses and neither clears the other. Set before each
    /// frame.
    pub fn set_held(&mut self, held: bool) {
        self.held = held;
    }

    /// Whether the menus pause the game ([`Self::set_paused`]) and nothing
    /// else: a control channel's hold aside.
    pub fn menu_paused(&self) -> bool {
        self.paused && self.pause_applies()
    }

    /// Whether the next frame is a paused one ([`Self::set_paused`],
    /// [`Self::set_held`]): no server tick is due. A pause holds only
    /// while the local player stands in the game and no Save and Exit is
    /// under way.
    pub fn is_paused(&self) -> bool {
        (self.paused || self.held) && self.pause_applies()
    }

    fn pause_applies(&self) -> bool {
        !self.world.exit_requested && self.world.local().is_some_and(|u| u.position.is_some())
    }

    /// A pass held by the app's draw pacing (`play --dump-draws`
    /// only): it counts as a frame and does
    /// nothing else: no pump, no receive, no update pass.
    pub fn held_frame(&mut self) {
        self.world.frames += 1;
    }

    /// One bridge frame: pump the server, receive and dispatch every
    /// delivered chunk, then, if the server ticked and
    /// the model is in game, the update pass, and
    /// the C→S messages the model answered with. A refused chunk ends the
    /// frame with an error; chunks after it in the same receive are not
    /// processed. The frame's mod messages go to the mod hook. The frame's
    /// outputs wait in the bridge for [`Self::take_outputs`]. A paused
    /// frame ([`Self::set_paused`]) runs only the skill fallback.
    pub fn frame(&mut self) -> Result<FrameReport, BridgeError> {
        self.frame_step(true)
    }

    fn frame_step(&mut self, clear_dialog_reply: bool) -> Result<FrameReport, BridgeError> {
        // A paused pass (UI state 9 or 11, the local player in a room): no pump (no
        // server frame), no receive, no update pass; only the skill
        // fallback, once. The pass still counts as a frame.
        // After Save and Exit the passes run until the server's answer:
        // no pause holds them.
        if self.is_paused() {
            self.world.frames += 1;
            let before = self.log.rejected.len();
            if let Err(error) = skill_fallback::skill_fallback(&mut self.world, &self.inputs) {
                self.log.rejected.push(receive::Rejected { id: 0, error });
            }
            return Ok(FrameReport {
                paused: true,
                rejected: self.log.rejected.len() - before,
                ..FrameReport::default()
            });
        }
        // A dialog-reply slot the UI layer did not answer after the last
        // frame's outputs carries no message (no case was handed back,
        // so no C→S 0x31).
        if clear_dialog_reply {
            self.world.outgoing.retain(|m| !m.is_empty());
        }
        // A resumed game stream (contracts 3.7): the server re-entered this
        // client and resends everything, so the model it was built on is
        // dropped before any of it is applied.
        if self.link.take_resume() {
            self.clear_after_resume();
        }
        self.ping()?;
        let pumped = self.link.pump()?;
        let mut report = FrameReport {
            ticked: pumped.ticked,
            ..FrameReport::default()
        };
        self.world.server_frame = self.link.server_frame();
        self.world.frames += 1;
        if pumped.ticked {
            self.world.server_ticks += 1;
        }
        let before = self.outputs.len();
        for chunk in self.link.receive() {
            let c = receive_chunk_on(
                &mut self.mods,
                &mut self.world,
                &self.inputs,
                &self.dispatch,
                &mut self.log,
                &mut self.outputs,
                &chunk,
            )?;
            report.chunks += 1;
            report.messages += c.messages;
            report.handled += c.handled;
            report.queued += c.queued;
            report.dropped += c.dropped;
            report.unowned += c.unowned;
            report.rejected += c.rejected;
            report.discarded_bytes += c.discarded_bytes;
        }
        self.run_mod_hook();
        if pumped.ticked && self.world.in_game {
            let before = self.log.rejected.len();
            // A moving skill's path step runs
            // the client's own step on the copied path words: the server
            // ran that step this tick, so its read is the step's result.
            let moving = critter_path::any_moving_skill(&self.world);
            // The local player's walk steps in its own update, before the
            // S monsters': this tick's server step.
            let steps = if moving || critter_path::local_walks(&self.world) {
                self.link.step_paths()
            } else {
                Vec::new()
            };
            if moving {
                for s in &steps {
                    critter_path::player_step_result(&mut self.world, s);
                }
                critter_path::knockback_without_server_path(&mut self.world, &steps);
            }
            report.drained = update::update_pass_steps(
                &mut self.world,
                &self.inputs,
                &self.dispatch,
                &mut self.log,
                &mut self.outputs,
                &steps,
            );
            // The last step of the drawn pass.
            if let Err(error) = skill_fallback::skill_fallback(&mut self.world, &self.inputs) {
                self.log.rejected.push(receive::Rejected { id: 0, error });
            }
            report.rejected += self.log.rejected.len() - before;
            // The single-player server copy at the end of the client
            // update.
            let copies = self.link.server_paths();
            self.server_copy |= !copies.is_empty();
            self.copy_server_paths(&copies);
        }
        report.answered = self.send_outgoing()?;
        output::move_freed(&mut self.world, &mut self.outputs);
        report.outputs = self.outputs.len() - before;
        Ok(report)
    }

    /// Advances a display frame, recovering up to sixteen overdue local ticks.
    /// Each tick receives and updates the client model before the next server
    /// tick. Presentation runs once afterwards. Tick-by-tick capture tools opt
    /// out to preserve their frame and input ordering.
    pub fn display_frame(&mut self, catch_up: bool) -> Result<(), BridgeError> {
        let paused = self.is_paused();
        let catch_up = catch_up && !std::mem::replace(&mut self.display_was_paused, paused);
        self.link.set_display_catch_up(catch_up);
        for step in 0..16 {
            // The UI answers dialogs after the display frame, so another
            // catch-up tick must not discard its still-unanswered reply slot.
            let report = self.frame_step(step == 0)?;
            self.sync_quest_flags();
            if !catch_up || !report.ticked || !self.link.tick_pending() {
                break;
            }
        }
        Ok(())
    }

    /// Applies one S→C chunk without pumping; its mod messages go to the
    /// mod hook.
    pub fn receive_chunk(&mut self, chunk: &[u8]) -> Result<receive::ChunkReport, BridgeError> {
        let report = receive_chunk_on(
            &mut self.mods,
            &mut self.world,
            &self.inputs,
            &self.dispatch,
            &mut self.log,
            &mut self.outputs,
            chunk,
        )?;
        self.run_mod_hook();
        Ok(report)
    }

    /// The update pass alone: drains every unit's queue.
    pub fn update_pass(&mut self) -> usize {
        update::update_pass(
            &mut self.world,
            &self.inputs,
            &self.dispatch,
            &mut self.log,
            &mut self.outputs,
        )
    }

    /// Hands the outputs made since the last call over, in order, and
    /// clears the list (once per frame, after the frame).
    pub fn take_outputs(&mut self) -> Vec<Output> {
        output::move_freed(&mut self.world, &mut self.outputs);
        self.sync_quest_flags();
        std::mem::take(&mut self.outputs)
    }

    fn sync_quest_flags(&mut self) {
        // The UI layer's client quest record that `ClientFn` 13 reads is
        // the last one a 0x28 (type 6) delivered. Before the first one the
        // record stays unset here, where a zeroed record would do (the
        // game UI start zeroes it and 0x28 copies Q for every T); without
        // one the invisible Ancient's update is fatal 0x1F every frame.
        if let Some(Output::QuestFlags { record }) = self
            .outputs
            .iter()
            .rev()
            .find(|o| matches!(o, Output::QuestFlags { .. }))
        {
            self.inputs.objclient.quest_flags = Some(*record);
        }
    }

    /// The outputs not yet handed over.
    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    /// Sends the model's own C→S messages through the send path, in
    /// order, and clears them, up to the
    /// first reserved dialog-reply slot (`world::DIALOG_REPLY_SLOT`):
    /// that slot and what follows wait for [`Self::npc_dialog_branch`].
    pub fn send_outgoing(&mut self) -> Result<usize, BridgeError> {
        let n = self
            .world
            .outgoing
            .iter()
            .position(|m| m.is_empty())
            .unwrap_or(self.world.outgoing.len());
        let out: Vec<Vec<u8>> = self.world.outgoing.drain(..n).collect();
        for m in &out {
            if m.first().is_some_and(|id| self.drop_own.contains(id)) {
                self.dropped.push(m.clone());
                continue;
            }
            self.send_bytes(m)?;
        }
        Ok(out.len())
    }

    /// Makes the model's own C→S messages with these ids be dropped
    /// instead of sent (not scripted injects).
    pub fn set_drop_own(&mut self, ids: Vec<u8>) {
        self.drop_own = ids;
    }

    /// The model's own messages dropped since the last call.
    pub fn take_dropped(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.dropped)
    }

    /// One world click ([`click`]): the dispatcher against the model, its
    /// C→S messages sent at once. Returns the outputs the UI layer applies
    /// (sounds, hover calls, the pending record) and the interact
    /// sender's outputs.
    pub fn world_click(
        &mut self,
        st: &mut crate::controls::click::ClickState,
        view: click::ClickView,
        kind: crate::controls::click::Kind,
        at: Option<(i32, i32)>,
        mods: u32,
    ) -> Result<(Vec<crate::controls::click::ClickOut>, Vec<output::Output>), BridgeError> {
        let r = click::world_click(&mut self.world, &self.inputs, st, view, kind, at, mods)
            .map_err(BridgeError::Click)?;
        self.send_outgoing()?;
        Ok(r)
    }

    /// The held repeat of a loop pass, its
    /// messages sent at once.
    pub fn click_repeat(
        &mut self,
        st: &mut crate::controls::click::ClickState,
        view: click::ClickView,
        mods: u32,
    ) -> Result<(Vec<crate::controls::click::ClickOut>, Vec<output::Output>), BridgeError> {
        let r = click::held_repeat(&mut self.world, &self.inputs, st, view, mods)
            .map_err(BridgeError::Click)?;
        self.send_outgoing()?;
        Ok(r)
    }

    /// [`Self::world_click`] with the local player read at `local_at`
    /// (`click::world_click_at`; the `play` preview's predicted position,
    /// decision D2, D2RUST's own, unverified).
    #[allow(clippy::too_many_arguments)]
    pub fn world_click_at(
        &mut self,
        st: &mut crate::controls::click::ClickState,
        view: click::ClickView,
        kind: crate::controls::click::Kind,
        at: Option<(i32, i32)>,
        mods: u32,
        local_at: Option<(u32, u32)>,
    ) -> Result<(Vec<crate::controls::click::ClickOut>, Vec<output::Output>), BridgeError> {
        let r = click::world_click_at(
            &mut self.world,
            &self.inputs,
            st,
            view,
            kind,
            at,
            mods,
            local_at,
        )
        .map_err(BridgeError::Click)?;
        self.send_outgoing()?;
        Ok(r)
    }

    /// The local player's mode as the model holds it (the mode a click may
    /// set).
    pub fn local_mode(&self) -> Option<u32> {
        let w = &self.world;
        w.units.get(&w.local_player?).map(|u| u.mode)
    }

    /// [`Self::click_repeat`] with the local player read at `local_at`
    /// (`click::held_repeat_at`).
    pub fn click_repeat_at(
        &mut self,
        st: &mut crate::controls::click::ClickState,
        view: click::ClickView,
        mods: u32,
        local_at: Option<(u32, u32)>,
    ) -> Result<(Vec<crate::controls::click::ClickOut>, Vec<output::Output>), BridgeError> {
        let r = click::held_repeat_at(&mut self.world, &self.inputs, st, view, mods, local_at)
            .map_err(BridgeError::Click)?;
        self.send_outgoing()?;
        Ok(r)
    }

    /// The client sender's clock for its duplicate filter
    /// ([`ServerLink::set_send_now`]): a check run sets the host clock
    /// its script fixes.
    pub fn set_send_now(&mut self, now: Option<u32>) {
        self.link.set_send_now(now);
    }

    /// The pending action of one client tick (`click::pending_step`),
    /// its messages sent at once.
    pub fn pending_step(
        &mut self,
        p: crate::controls::click::Pending,
        run: bool,
        local_at: Option<(u32, u32)>,
    ) -> Result<(click::PendStep, Vec<output::Output>), BridgeError> {
        // The pending action runs in its own place, before the update:
        // not a shifted script click, so its
        // request takes no late player step ([`world::ClientWorld::late_input`]).
        let late = std::mem::replace(&mut self.world.late_input, false);
        let r = click::pending_step(&mut self.world, &self.inputs, p, run, local_at);
        self.world.late_input = late;
        let r = r.map_err(BridgeError::Click)?;
        self.send_outgoing()?;
        Ok(r)
    }

    /// The interact sender on `key`, its
    /// messages sent at once (the `play` preview's pending interaction on
    /// arrival, `world_view::interact`; D2RUST's own, unverified).
    pub fn interact(&mut self, key: UnitKey) -> Result<Vec<output::Output>, BridgeError> {
        let out = objects::interact::send(
            &mut self.world,
            &self.inputs,
            u16::from(key.unit_type),
            key.guid,
        )
        .map_err(BridgeError::Click)?;
        self.send_outgoing()?;
        Ok(out)
    }

    /// The play preview's pending action on arrival ("pend"): code 0x13
    /// is the interact sender ([`Self::interact`]); a skill code (6: the
    /// left skill on the unit, 0xD: the right) is that action itself
    /// (code, player, type, GUID): the local mode request and the
    /// C→S message, sent at once. Before B024 every pending code ran the
    /// interact sender, so an attack that had to walk first never came.
    pub fn pending_action(
        &mut self,
        p: crate::controls::click::Pending,
    ) -> Result<Vec<output::Output>, BridgeError> {
        use crate::controls::click::{ClickOut, CODE_INTERACT};
        if p.code == CODE_INTERACT {
            return self.interact(p.target);
        }
        let (_, out) = click::apply(
            &mut self.world,
            &self.inputs,
            vec![ClickOut::Code {
                code: p.code,
                a: u32::from(p.target.unit_type),
                b: p.target.guid,
            }],
        )
        .map_err(BridgeError::Click)?;
        self.send_outgoing()?;
        Ok(out)
    }

    /// The UI layer's answer to an `NpcDialog` output: the bridge applies
    /// the branch's model writes and C→S 0x31 in order
    /// ([`msg::ui_npc::apply_dialog_branch`]), then sends the
    /// messages that waited behind the slot. Returns how many were sent.
    pub fn npc_dialog_branch(
        &mut self,
        dialog: &output::NpcDialog,
        case: msg::ui_npc::DialogCase,
    ) -> Result<usize, BridgeError> {
        msg::ui_npc::apply_dialog_branch(&mut self.world, dialog, case);
        self.send_outgoing()
    }

    /// The draw's Y sort of a room's unit list written back to the client
    /// list (the sorted order persists).
    /// `false`: `order` is not a permutation of the list (nothing changed).
    pub fn set_room_order(&mut self, room: drlg::DrlgRoomId, order: &[UnitKey]) -> bool {
        self.world.room_units.set_order(room, order)
    }

    /// The drawn frame's unit draws take their hit flash counters
    /// ([`ClientWorld::take_hit_flashes`]).
    pub fn take_hit_flashes(&mut self, drawn: &[UnitKey]) {
        self.world.take_hit_flashes(drawn);
    }

    /// The tables the message rules read.
    pub fn set_tables(&mut self, tables: ClientTables) {
        self.inputs.tables = tables;
        self.sync_corpse_tables();
    }

    /// The hover's corpse test reads its tables from the model
    /// ([`hover::CorpseTables`]).
    fn sync_corpse_tables(&mut self) {
        self.world.corpse_tables = hover::CorpseTables::of(&self.inputs.tables);
    }

    /// The `Levels.txt` rows; the other tables
    /// stay (the skill rows bound earlier survive).
    pub fn set_levels(&mut self, levels: Vec<world::LevelRow>) {
        self.inputs.tables.levels = levels;
    }

    /// The unit-message rows: `monstats` / `monstats2`, `itemstatcost` send
    /// columns, `objects.txt` and `shrines.txt`; the other tables stay.
    pub fn set_unit_rows(&mut self, rows: world::UnitRows) {
        let t = &mut self.inputs.tables;
        self.world.objclient.monster_pieces = rows
            .monsters
            .iter()
            .map(|m| m.map_or(0, |m| m.total_pieces))
            .collect();
        t.monsters = rows.monsters;
        t.monster_skill_bonus = rows.monster_skill_bonus;
        t.stats = rows.stats;
        t.objects = rows.objects;
        t.shrines = rows.shrines;
        t.states = rows.states;
        t.missiles = rows.missiles;
        t.overlays = rows.overlays;
        t.monster_sequences = rows.monster_sequences;
        t.hirelings = rows.hirelings;
        self.sync_corpse_tables();
    }

    /// The host's wall-clock seconds (the light flicker reads them).
    pub fn set_wall_seconds(&mut self, f: fn() -> i32) {
        self.inputs.wall_seconds = Some(f);
    }

    /// Whether the install has the expansion data (`d2exp.mpq`): 0x97
    /// flips the weapon set only with it (playtest B021).
    pub fn set_expansion_installed(&mut self, installed: bool) {
        self.inputs.expansion_installed = installed;
    }

    /// The light quality is on (≠ 0): client missiles get lights.
    pub fn set_high_light_quality(&mut self, high: bool) {
        self.inputs.high_light_quality = high;
    }

    /// The skills tables of the passive refresh.
    pub fn set_skill_tables(&mut self, tables: std::sync::Arc<d2_sim::skills::SkillTables>) {
        self.inputs.skill_tables = Some(tables);
    }

    /// The players' animation lookup of the client player update
    /// ([`player_anim`]).
    pub fn set_player_anims(&mut self, anims: std::sync::Arc<dyn player_anim::PlayerAnims>) {
        self.world.tip_anims = player_anim::PlayerAnimsRef(Some(anims.clone()));
        self.inputs.player_anims = Some(anims);
    }

    /// The `skills` rows of the client skill list; the other tables stay.
    pub fn set_skill_rows(&mut self, rows: Vec<world::SkillRow>) {
        self.inputs.tables.skills = rows;
        self.sync_corpse_tables();
    }

    /// Each class's `charstats` Skill 1–10;
    /// the other tables stay.
    /// Each class's charstats life / stamina per vitality and mana per
    /// energy (the full array of the local player's maximums).
    pub fn set_class_vitals(&mut self, v: Vec<[u8; 3]>) {
        self.world.class_vitals = v;
    }

    /// Each stat's op-target table (the percent ops of the full array).
    pub fn set_stat_ops(&mut self, ops: Vec<Vec<(u16, u8)>>) {
        self.world.stat_ops = ops;
    }

    pub fn set_class_skills(&mut self, class_skills: Vec<[u16; 10]>) {
        self.inputs.tables.class_skills = class_skills;
    }

    /// Each class's `charstats` walk and run velocity.
    pub fn set_class_velocities(&mut self, v: Vec<[u8; 2]>) {
        self.inputs.tables.class_velocities = v;
    }

    /// What the client DRLG of 0x03 is built from; `None`: no client DRLG.
    pub fn set_drlg_source(&mut self, source: Option<drlg::DrlgSource>) {
        self.inputs.drlg = source;
    }

    /// The wall clock of the next update, in wrapping milliseconds: the
    /// live client passes the host clock, tests a scripted value.
    pub fn set_now(&mut self, now: u32) {
        self.inputs.now = now;
    }

    /// The monster graphics-ready flags by drawn tick: a check run's
    /// scripted host input; `None`: not given.
    pub fn set_gfx_ready(
        &mut self,
        ready: Option<std::sync::Arc<std::collections::BTreeMap<u64, world::GfxReady>>>,
    ) {
        self.world.gfx_ready = ready;
    }

    /// The `objects.txt` rows the client object update reads; empty: no
    /// object update.
    pub fn set_object_rows(&mut self, rows: Vec<objects::ObjClientRow>) {
        self.world.objclient.selectable = rows.iter().map(|r| r.selectable).collect();
        self.world.objclient.hover_layers = rows.iter().map(hover::ObjectLayerTest::of).collect();
        self.inputs.objclient.rows = rows;
    }

    /// The UI layer's client quest record (`ClientFn` 13 reads it).
    pub fn set_client_quest_flags(&mut self, flags: Option<[u8; objects::QUEST_RECORD]>) {
        self.inputs.objclient.quest_flags = flags;
    }

    /// The visibility predicate of the position check.
    pub fn set_visibility(&mut self, visible: Option<VisibleFn>) {
        self.inputs.visible = visible;
    }

    /// The view's layer cel offsets (the unit composite and the imp
    /// teleport's client function read them).
    pub fn set_cel_offset(&mut self, cel_offset: Option<world::CelOffsetFn>) {
        self.inputs.cel_offset = cel_offset;
    }

    pub fn inputs(&self) -> &ModelInputs {
        &self.inputs
    }

    /// The single-player server copy for the server monsters in `units`,
    /// run by a host that holds
    /// the in-process server's units after a tick.
    /// A walking player's client step aims where the server's did this
    /// tick and the update turns it:
    /// both run here, on the copy's read of the server (`new_dir`).
    pub fn copy_server_paths(&mut self, units: &[critter_path::ServerPath]) {
        for s in units {
            if self.local_aim_late && self.world.local_player == Some(s.key) {
                self.held_aim = Some(s.clone());
                continue;
            }
            critter_path::player_aim(&mut self.world, s);
        }
        for s in units {
            critter_path::server_copy(&mut self.world, &self.inputs, s);
        }
    }

    /// The local player's aim waits for the play preview
    /// ([`Bridge::aim_local`]): on a walk to a unit the client re-plans
    /// its path every tick before the update, so the update aims along
    /// the client plan, which the preview holds.
    /// `state-dump` aims in its own copy instead (`app::state_dump`).
    pub fn set_local_aim_late(&mut self, late: bool) {
        self.local_aim_late = late;
    }

    /// The held aim of the local player ([`Bridge::set_local_aim_late`])
    /// with `new_dir`, the new direction of the previous frame's client
    /// plan of a walk to a unit (`None`: the server's stands).
    pub fn aim_local(&mut self, new_dir: Option<u8>) {
        if let Some(mut s) = self.held_aim.take() {
            if new_dir.is_some() {
                s.new_dir = new_dir;
            }
            critter_path::player_aim(&mut self.world, &s);
        }
    }

    /// The script's clicks of this pass come after the update, where the
    /// live client runs them before it ([`world::ClientWorld::late_input`]).
    pub fn set_late_input(&mut self, late: bool) {
        self.world.late_input = late;
    }

    /// The model, writable (tests only).
    #[cfg(test)]
    pub(crate) fn world_mut(&mut self) -> &mut ClientWorld {
        &mut self.world
    }

    /// The local player's predicted precise position
    /// ([`ClientWorld::local_predicted`]), set by the play preview each
    /// frame.
    pub fn set_local_predicted(&mut self, at: Option<(u32, u32)>) {
        self.world.local_predicted = at;
        critter_path::refresh_local_faces(&mut self.world);
    }

    /// Restarts the local player's neutral animation (`player_anim::mode_init`
    /// in its current model mode): the frame returns to the mode's start
    /// bonus, as the client does when the walk ends and the town
    /// neutral mode is entered again. A presentation hook of the play
    /// preview; checked by `bugs-d-b002-walk-far-cs` and `walk-hold-ama`.
    /// A walk ended by a cast, attack or hit (a mode outside neutral and
    /// walk, which includes [`player_anim::ENDING_MODES`]) keeps that
    /// mode's animation: its set restarted it already, and it runs on
    /// from there (`fx-nova-cast`: cast frame 1 at tick 265, 2 at 266;
    /// `fx-ice-bolt-flight`: a restart left the cast one update late and
    /// the next click refused).
    pub fn restart_local_neutral(&mut self) {
        let Some(local) = self.world.local() else {
            return;
        };
        let (key, mode) = (local.key, local.mode);
        if !matches!(mode, 1 | 2 | 3 | 5 | 6) {
            return;
        }
        player_anim::mode_init(&mut self.world, &self.inputs, key, mode);
    }

    /// The play preview's own-walk room recache of the local player at
    /// the predicted sub-tile ([`ClientWorld::recache_local_room`]);
    /// checked by `bugs-d-b002-walk-far-cs` and `walk-hold-ama`.
    pub fn recache_local_room(&mut self, x: u16, y: u16) -> bool {
        self.world.recache_local_room(x, y)
    }

    /// The play preview's predicted precise position of the local player
    /// and its walk / run mode, for the position check
    /// ([`ClientWorld::set_local_walk`], [`ClientWorld::predicted`]);
    /// checked by `bugs-d-b002-walk-far-cs` and `walk-hold-ama`.
    ///
    /// A prediction left behind by a server teleport or an act load
    /// ([`ClientWorld::is_stale_prediction`]) is dropped, so the server's
    /// placement stands.
    pub fn set_local_walk(&mut self, pos: Option<(u32, u32)>, mode: Option<u32>) {
        let pos = pos.filter(|&p| !self.world.is_stale_prediction(p));
        self.world.set_local_walk(pos, mode);
    }

    /// The local player's path direction (`dir64`, the cstate dump's `d`).
    pub fn set_local_dir(&mut self, dir: u8) {
        if let Some(k) = self.world.local_player {
            if let Some(u) = self.world.units.get_mut(&k) {
                u.path_dir = Some(dir);
            }
        }
    }

    /// Sets the local player's unit seed {lo, hi} (`ClientUnit::seed`):
    /// `state-dump --player-seed` sets a scripted value per frame.
    pub fn set_local_seed(&mut self, lo: u32, hi: u32) {
        if let Some(k) = self.world.local_player {
            if let Some(u) = self.world.units.get_mut(&k) {
                u.seed = Some((lo, hi));
            }
        }
    }

    /// Installs the item tables the model decodes item streams with.
    pub fn set_item_tables(&mut self, t: std::sync::Arc<dyn item_lists::StreamProps>) {
        self.world.set_item_tables(t);
    }

    pub fn world(&self) -> &ClientWorld {
        &self.world
    }

    /// Takes the model's hit overlay creates (`hit_overlays`) for the
    /// effect layer, in order.
    pub fn take_hit_overlays(&mut self) -> Vec<hit_overlays::HitOverlay> {
        std::mem::take(&mut self.world.hit_overlays)
    }

    /// A drawn frame's camera: the unit origin the client missile
    /// function 2 reads (`world::UnitOrigin`).
    pub fn set_unit_origin(&mut self, cam: &crate::rules::camera::Camera) {
        self.world.unit_origin = Some(world::UnitOrigin::of(cam));
    }

    /// A drawn frame's light pass: `pass` gets
    /// the model and the client's kept light list, taken out of the
    /// model for the pass and put back after it.
    pub fn light_frame(&mut self, pass: impl FnOnce(&ClientWorld, &mut LightList)) {
        let mut lights = std::mem::take(&mut self.world.lights);
        pass(&self.world, &mut lights);
        self.world.lights = lights;
    }

    /// The client DRLG, writable: a test seam for fixture collision (a
    /// wall the fixture tiles lack, stamped in the server's grid too).
    /// Nothing in the client writes the DRLG through it.
    #[doc(hidden)]
    pub fn drlg_mut(&mut self) -> Option<&mut drlg::ClientDrlg> {
        self.world.drlg.as_mut()
    }

    /// The client `skills` row of `skill`, when the tables have one.
    pub fn skill_row(&self, skill: u16) -> Option<world::SkillRow> {
        self.inputs.tables.skills.get(usize::from(skill)).copied()
    }

    /// The play preview's monster motion on the model (D2RUST's own,
    /// unverified; [`motion`]).
    pub fn preview_motion(&mut self, m: &mut motion::MonsterMotion) {
        if !self.server_copy {
            for key in m.frame(&mut self.world, &self.inputs.tables.monsters) {
                modes::end_walk(&mut self.world, &self.inputs, key);
            }
        }
    }

    pub fn log(&self) -> &ReceiveLog {
        &self.log
    }

    pub fn link(&self) -> &L {
        &self.link
    }

    pub fn link_mut(&mut self) -> &mut L {
        &mut self.link
    }
}

impl<L: ServerLink + ?Sized> ServerLink for Box<L> {
    fn protocol_version(&self) -> u32 {
        (**self).protocol_version()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        (**self).tick_rate()
    }

    fn server_frame(&mut self) -> Option<i32> {
        (**self).server_frame()
    }

    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        (**self).send(queue, msg)
    }

    fn pump(&mut self) -> Result<Pumped, LinkError> {
        (**self).pump()
    }

    fn set_display_catch_up(&mut self, enabled: bool) {
        (**self).set_display_catch_up(enabled)
    }

    fn tick_pending(&mut self) -> bool {
        (**self).tick_pending()
    }

    fn receive(&mut self) -> Vec<Vec<u8>> {
        (**self).receive()
    }

    fn server_paths(&mut self) -> Vec<critter_path::ServerPath> {
        (**self).server_paths()
    }

    fn step_paths(&mut self) -> Vec<critter_path::ServerPath> {
        (**self).step_paths()
    }

    fn is_remote(&self) -> bool {
        (**self).is_remote()
    }

    fn take_resume(&mut self) -> bool {
        (**self).take_resume()
    }

    fn set_send_now(&mut self, now: Option<u32>) {
        (**self).set_send_now(now)
    }
}

#[cfg(test)]
mod client_rate_tests;
