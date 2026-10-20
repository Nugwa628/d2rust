//! The S→C handlers of the client world model, one per owned id of
//! `bridge-dispatch.tsv`, registered in [`HANDLERS`]:
//!
//! - [`session`]: (0x00–0x08, 0x0B, 0xB4);
//! - [`pets`]: (0x7A, 0x81);
//! - [`units`]: (unit add, remove, place, queued
//!   movement and action messages with the client model, the local
//!   player's vitals, 0x09);
//! - [`unit_misc`], [`roster`], [`states`]:–;
//! - [`stats_items`], [`items`]:;
//! - [`skills`];
//! - [`ui`], [`ui_text`], [`ui_npc`], [`ui_quest`]:
//!   (outputs to the UI and their model parts);
//! - [`sound`]: (0x2C: output to the audio);
//! - [`lighting`]: (0x53, 0x89);
//! - [`no_op`]: the no-op ids (step 6).

pub mod items;
pub mod lighting;
pub mod pets;
pub mod roster;
pub mod session;
pub mod skills;
pub mod sound;
pub mod states;
pub mod stats_items;
pub mod ui;
pub mod ui_npc;
pub mod ui_quest;
pub mod ui_text;
pub mod unit_misc;
pub mod units;

#[cfg(test)]
mod tests_c_client;
#[cfg(test)]
mod tests_drlg;
#[cfg(test)]
mod tests_items_skills;
#[cfg(test)]
mod tests_model;
#[cfg(test)]
mod tests_outputs;
#[cfg(test)]
mod tests_pc1;
#[cfg(test)]
mod tests_proto_a;
#[cfg(test)]
mod tests_proto_b;
#[cfg(test)]
mod tests_proto_c;
#[cfg(test)]
mod tests_proto_d;
#[cfg(test)]
mod tests_proto_e;
#[cfg(test)]
mod tests_stats_items;
#[cfg(test)]
mod tests_ui_more;
#[cfg(test)]
mod tests_units;
#[cfg(test)]
mod tests_units_more;

use super::dispatch::{
    Handle, Handler, HandlerError, HandlerFn, Message, UnitHandlerFn, UnitMessage,
};
use super::world::ClientWorld;

pub const MODEL: &str = "specs/client/model.md";
pub const UNITS: &str = "specs/client/msg-units.md";
pub const STATS_ITEMS: &str = "specs/client/msg-stats-items.md";
pub const SKILLS: &str = "specs/client/msg-skills.md";
pub const UI: &str = "specs/client/msg-ui.md";
pub const TRIGGERS: &str = "specs/audio/triggers.md";
pub const LIGHTING: &str = "specs/render/lighting.md";
pub const BRIDGE: &str = "specs/client/bridge.md";

const fn general(id: u8, owner: &'static str, f: HandlerFn) -> Handler {
    Handler {
        id,
        owner,
        handle: Handle::General(f),
    }
}

const fn unit(id: u8, f: UnitHandlerFn) -> Handler {
    Handler {
        id,
        owner: UNITS,
        handle: Handle::Unit(f),
    }
}

/// The shared no-op handler of the no-op ids: size-0
/// ids (never called), bare-`ret` handlers and never-produced ids.
pub fn no_op(_: &mut ClientWorld, _: &Message<'_>) -> Result<(), HandlerError> {
    Ok(())
}

/// The no-op of 0x17, the one id with a receive-table unit
/// handler (size 0: never called).
pub fn no_op_unit(_: &mut ClientWorld, _: &UnitMessage<'_>) -> Result<(), HandlerError> {
    Ok(())
}

const fn none(id: u8) -> Handler {
    general(id, BRIDGE, no_op)
}

/// Every owned id's handler, in id order.
pub const HANDLERS: &[Handler] = &[
    general(0x00, MODEL, session::game_loading),
    general(0x01, MODEL, session::game_flags),
    general(0x02, MODEL, session::load_successful),
    general(0x03, MODEL, session::load_act),
    general(0x04, MODEL, session::load_complete),
    general(0x05, MODEL, session::unload_complete),
    general(0x06, MODEL, session::game_exit),
    general(0x07, MODEL, session::map_reveal),
    general(0x08, MODEL, session::map_hide),
    general(0x09, UNITS, units::assign_level_warp),
    general(0x0A, UNITS, units::remove_unit),
    general(0x0B, MODEL, session::game_handshake),
    unit(0x0C, units::queued),
    unit(0x0D, units::queued),
    unit(0x0E, units::queued),
    unit(0x0F, units::queued),
    unit(0x10, units::queued),
    general(0x11, UNITS, unit_misc::unit_overlay),
    none(0x12),
    none(0x13),
    none(0x14),
    general(0x15, UNITS, units::reassign_player),
    none(0x16),
    Handler {
        id: 0x17,
        owner: BRIDGE,
        handle: Handle::Unit(no_op_unit),
    },
    general(0x18, UNITS, units::vitals),
    general(0x19, STATS_ITEMS, stats_items::local_stat),
    general(0x1A, STATS_ITEMS, stats_items::local_stat),
    general(0x1B, STATS_ITEMS, stats_items::local_stat),
    general(0x1C, STATS_ITEMS, stats_items::local_stat),
    general(0x1D, STATS_ITEMS, stats_items::local_stat),
    general(0x1E, STATS_ITEMS, stats_items::local_stat),
    general(0x1F, STATS_ITEMS, stats_items::local_stat),
    general(0x20, STATS_ITEMS, stats_items::stat_update),
    general(0x21, SKILLS, skills::update_item_oskill),
    general(0x22, SKILLS, skills::update_item_skill),
    general(0x23, SKILLS, skills::set_skill),
    none(0x24),
    none(0x25),
    general(0x26, UI, ui_text::chat),
    general(0x27, UI, ui_text::npc_text),
    general(0x28, UI, ui_npc::quest_info),
    general(0x29, UI, ui_quest::game_quest_flags),
    general(0x2A, UI, ui_npc::npc_transaction),
    none(0x2B),
    general(0x2C, TRIGGERS, sound::play_sound),
    none(0x2D),
    none(0x2E),
    none(0x2F),
    none(0x30),
    none(0x31),
    none(0x32),
    none(0x33),
    none(0x34),
    none(0x35),
    none(0x36),
    none(0x37),
    none(0x38),
    none(0x39),
    none(0x3A),
    none(0x3B),
    none(0x3C),
    none(0x3D),
    general(0x3E, STATS_ITEMS, items::update_item_stats),
    general(0x3F, STATS_ITEMS, stats_items::use_stackable_item),
    general(0x40, STATS_ITEMS, items::item_flags),
    none(0x41),
    general(0x42, STATS_ITEMS, stats_items::clear_cursor),
    none(0x43),
    none(0x44),
    none(0x45),
    none(0x46),
    general(0x47, STATS_ITEMS, stats_items::relator),
    general(0x48, STATS_ITEMS, stats_items::relator),
    none(0x49),
    none(0x4A),
    none(0x4B),
    unit(0x4C, units::queued),
    unit(0x4D, units::queued),
    general(0x4E, UI, ui_npc::hire_offer),
    general(0x4F, UI, ui_npc::hire_list_reset),
    general(0x50, UI, ui_quest::quest_special),
    general(0x51, UNITS, units::assign_object),
    general(0x52, UI, ui_quest::quest_log),
    general(0x53, LIGHTING, lighting::darkness),
    none(0x54),
    none(0x55),
    none(0x56),
    general(0x57, UNITS, unit_misc::npc_enchants),
    general(0x58, UI, ui_quest::open_ui),
    general(0x59, UNITS, units::assign_player),
    general(0x5A, UI, ui_text::event_text),
    general(0x5B, UNITS, roster::player_joined),
    general(0x5C, UNITS, roster::player_left),
    general(0x5D, UI, ui::quest_status),
    general(0x5E, UI, ui_quest::quest_availability),
    general(0x5F, UNITS, unit_misc::portal_flags),
    general(0x60, UNITS, unit_misc::town_portal_state),
    general(0x61, UI, ui_text::act_video),
    general(0x62, UI, ui_npc::dialog_end),
    general(0x63, UI, ui::waypoint_menu),
    none(0x64),
    general(0x65, UNITS, roster::player_kill_count),
    none(0x66),
    unit(0x67, units::queued),
    unit(0x68, units::queued),
    unit(0x69, units::queued),
    unit(0x6A, units::queued),
    unit(0x6B, units::queued),
    unit(0x6C, units::queued),
    unit(0x6D, units::queued),
    unit(0x6E, units::no_effect),
    unit(0x6F, units::no_effect),
    unit(0x70, units::no_effect),
    unit(0x71, units::no_effect),
    unit(0x72, units::no_effect),
    general(0x73, UNITS, unit_misc::client_missile),
    general(0x74, UNITS, unit_misc::player_corpse_assign),
    general(0x75, UNITS, roster::player_party_info),
    general(0x76, UI, ui_text::overhead_clear),
    general(0x77, UI, ui::trade_action),
    general(0x78, UI, ui_text::trade_partner),
    general(0x79, UI, no_op),
    general(0x7A, MODEL, pets::pet_action),
    general(0x7B, UI, ui_text::hotkey),
    general(0x7C, STATS_ITEMS, items::use_scroll),
    general(0x7D, STATS_ITEMS, items::set_item_state),
    general(0x7E, UNITS, unit_misc::common_cof),
    general(0x7F, UNITS, no_op),
    none(0x80),
    general(0x81, MODEL, pets::assign_merc),
    general(0x82, UNITS, roster::portal_ownership),
    none(0x83),
    none(0x84),
    none(0x85),
    none(0x86),
    none(0x87),
    none(0x88),
    general(0x89, LIGHTING, lighting::unique_event),
    general(0x8A, UI, ui_npc::npc_interact),
    general(0x8B, UNITS, no_op),
    general(0x8C, UNITS, no_op),
    general(0x8D, UNITS, no_op),
    general(0x8E, UNITS, roster::corpse_assign),
    general(0x8F, MODEL, session::pong),
    general(0x90, UNITS, no_op),
    general(0x91, UI, ui_npc::npc_intro),
    general(0x92, STATS_ITEMS, items::remove_items_display),
    general(0x93, SKILLS, skills::skill_bonus),
    general(0x94, SKILLS, skills::base_skill_levels),
    general(0x95, UNITS, units::vitals),
    general(0x96, UNITS, units::vitals),
    general(0x97, STATS_ITEMS, items::weapon_switch),
    general(0x98, UNITS, unit_misc::monster_f40),
    general(0x99, SKILLS, skills::skill_event_unit),
    general(0x9A, SKILLS, skills::skill_event_point),
    general(0x9B, UI, ui_npc::merc_revive),
    general(0x9C, STATS_ITEMS, stats_items::item_action),
    general(0x9D, STATS_ITEMS, stats_items::item_action),
    general(0x9E, STATS_ITEMS, items::merc_stat),
    general(0x9F, STATS_ITEMS, items::merc_stat),
    general(0xA0, STATS_ITEMS, items::merc_stat),
    general(0xA1, STATS_ITEMS, items::merc_stat),
    general(0xA2, STATS_ITEMS, items::merc_stat),
    general(0xA3, SKILLS, skills::skill_do),
    general(0xA4, UNITS, unit_misc::baal_wave),
    general(0xA5, SKILLS, skills::skill_end),
    general(0xA6, STATS_ITEMS, items::item_table_entry),
    general(0xA7, UNITS, states::delayed_or_end_state),
    general(0xA8, UNITS, states::set_state),
    general(0xA9, UNITS, states::delayed_or_end_state),
    general(0xAA, UNITS, states::add_unit_states),
    general(0xAB, UNITS, unit_misc::npc_heal),
    general(0xAC, UNITS, units::assign_monster),
    none(0xAD),
    general(0xAE, BRIDGE, no_op),
    general(0xAF, MODEL, session::connection_info),
    general(0xB0, MODEL, session::connection_terminated),
    none(0xB1),
    none(0xB2),
    general(0xB3, MODEL, no_op),
    general(0xB4, MODEL, session::join_refused),
    general(0xB6, MODEL, session::view_ready),
];

/// Little-endian field reads with a handler error for a short message.
pub(crate) struct Bytes<'a>(pub &'a [u8]);

impl Bytes<'_> {
    fn get(&self, off: usize, n: usize) -> Result<&[u8], HandlerError> {
        self.0
            .get(off..off + n)
            .ok_or(HandlerError::Invalid("message too short for its layout"))
    }

    pub fn u8(&self, off: usize) -> Result<u8, HandlerError> {
        Ok(self.get(off, 1)?[0])
    }

    pub fn u16(&self, off: usize) -> Result<u16, HandlerError> {
        let b = self.get(off, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&self, off: usize) -> Result<u32, HandlerError> {
        let b = self.get(off, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn slice(&self, off: usize, n: usize) -> Result<&[u8], HandlerError> {
        self.get(off, n)
    }
}

/// Test support: the dispatch over a bare model.
#[cfg(test)]
pub(crate) mod support {
    use super::super::dispatch::Dispatch;
    use super::super::output::Output;
    use super::super::receive::{receive_chunk, ReceiveLog};
    use super::super::update::update_pass;
    use super::super::world::{ClientUnit, ClientWorld, ModelInputs, UnitKey};

    /// A model, its inputs and the receive log, driven by the dispatch
    /// table.
    #[derive(Default)]
    pub struct Model {
        pub w: ClientWorld,
        pub inputs: ModelInputs,
        pub log: ReceiveLog,
        /// The handlers' outputs, in order.
        pub out: Vec<Output>,
    }

    /// Bytes from a hex string ("6d 06 00 …").
    pub fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    impl Model {
        /// Receives `bytes` as one chunk; panics if the chunk is refused.
        pub fn recv(&mut self, bytes: &[u8]) -> &mut Self {
            let d = Dispatch::from_spec().unwrap();
            receive_chunk(
                &mut self.w,
                &self.inputs,
                &d,
                &mut self.log,
                &mut self.out,
                bytes,
            )
            .unwrap();
            self
        }

        /// Receives the hex message.
        pub fn hex(&mut self, s: &str) -> &mut Self {
            self.recv(&hex(s))
        }

        /// Runs the update pass; returns the messages applied.
        pub fn drain(&mut self) -> usize {
            let d = Dispatch::from_spec().unwrap();
            update_pass(&mut self.w, &self.inputs, &d, &mut self.log, &mut self.out)
        }

        /// Inserts a bare unit (fixture state).
        pub fn put(&mut self, key: UnitKey) -> &mut ClientUnit {
            self.w.units.insert(key, ClientUnit::new(key));
            self.w.units.get_mut(&key).unwrap()
        }

        pub fn unit(&self, key: UnitKey) -> &ClientUnit {
            &self.w.units[&key]
        }

        /// The ids the handlers rejected, in order.
        pub fn rejected(&self) -> Vec<(u8, String)> {
            self.log
                .rejected
                .iter()
                .map(|r| (r.id, r.error.to_string()))
                .collect()
        }
    }
}
