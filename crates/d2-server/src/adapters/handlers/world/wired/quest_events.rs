//! The quest events of the play host (a child module of the wired host,
//! [`super::WiredWorld`]): monster init's chain links and monster deaths,
//! which the action wiring's seams queue ([`Pending::take_quest_events`]).
//! They run on the quest control after the tick (`after_tick`).
//! The players' level changes (quest event 3) run in the tick's per-client
//! update (the action wiring's `client_level_change` on the lent quest
//! control).
//!
//! The links of monster init and the kill parse run inside their caller
//! on the lent quest control (`View::quest_link_now`,
//! `View::quest_kill_now`); what reaches this queue comes from a call with
//! no quest control lent and runs once per tick after the tick's steps,
//! links before kills. The in-place timing was measured: the item-drop and
//! quest checks show no difference between running there and here.

use crate::adapters::handlers::world::Outbox;
use d2_sim::game::Game;
use d2_sim::units::UnitType;
use d2_sim::wiring::action::{Pending, QuestEvent};
use d2_sim::wiring::economy::quest_objects::link_items;
use d2_sim::world::quests::{act2, act3, act5, QuestWorld};

use super::{quest_call, ActionEvents, TradeRest, WiredWorld};

impl<R: TradeRest, S> WiredWorld<R, S> {
    /// The cube's `hst ` hook (the Horadric Staff assembled), run
    /// when the 0x4F handler returns: the game calls it inside the
    /// transmute, so its S→C 0x28 follows the removal messages of the
    /// same handler, ahead of the next tick's item pass.
    pub(super) fn run_cube_staff<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D) {
        let Some(cube) = self.cube.as_mut() else {
            return;
        };
        let players: Vec<_> = cube
            .pending
            .take_quest_items()
            .into_iter()
            .filter(|(_, code)| *code == *b"hst ")
            .map(|(player, _)| player)
            .collect();
        if players.is_empty() {
            return;
        }
        self.desk(game, events, |desk, ctl, inv| {
            let ((), _) = quest_call(desk, ctl, inv, |q, w| {
                link_items(q, w);
                for &player in &players {
                    act2::q2::staff_assembled(q, w, player);
                }
            });
        });
    }

    /// The kill parse of the monsters killed in the
    /// tick's unit steps, with the chain links queued before them, at the
    /// end of tick step 4 (before the client pass): the game runs it
    /// inside the kill, after the treasure drop, so a quest drop (Radament's
    /// `ass `, Hephasto's `hfh `) is created and queued for update in the
    /// tick of the kill and announced in its client pass, ahead of the
    /// treasure drop's items.
    /// The other queued events go back to the queue for
    /// [`Self::run_quest_events`] after the tick. Only kills queued with no
    /// quest control lent reach this (see the module note).
    pub(super) fn run_kill_events<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D) {
        let queued = events.action().sys.hooks.x.take_quest_events();
        if !queued.iter().any(|e| matches!(e, QuestEvent::Kill { .. })) {
            for e in queued {
                events.action().sys.hooks.x.queue_quest_event(e);
            }
            return;
        }
        let (early, rest): (Vec<_>, Vec<_>) = queued
            .into_iter()
            .partition(|e| matches!(e, QuestEvent::Link { .. } | QuestEvent::Kill { .. }));
        for e in rest {
            events.action().sys.hooks.x.queue_quest_event(e);
        }
        self.desk(game, events, |desk, ctl, inv| {
            let ((), _) = quest_call(desk, ctl, inv, |q, w| {
                link_items(q, w);
                for e in &early {
                    if let QuestEvent::Link { unit, chain } = *e {
                        q.add_link(w, unit, chain, None);
                    }
                }
                for e in &early {
                    if let QuestEvent::Kill { victim, killer } = *e {
                        monster_killed(q, w, victim, killer);
                    }
                }
            });
        });
    }

    /// Hook ITEMPICKEDUP (event 4 to the item's chain) for the
    /// ground items a move call picked up, run at the end of that call:
    /// the game raises it inside the pick-up, so the quest's status message
    /// goes out in the input pass, before the tick's removal of the ground
    /// item (the message is `5D 03 00 02 00 00`).
    pub(super) fn run_item_picks<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D)
    where
        D::X: Outbox,
    {
        let picks = std::mem::take(&mut self.item_picks);
        if picks.is_empty() {
            return;
        }
        self.desk(game, events, |desk, ctl, inv| {
            let ((), _) = quest_call(desk, ctl, inv, |q, w| {
                link_items(q, w);
                for &(player, item) in &picks {
                    // Item creation links an item with `quest` ≠ 0
                    // to chain `quest` − 1; its link is
                    // applied by `link_items` above. An item not made by
                    // `Economy::create_item` (none on this host) is linked here.
                    let quest = w
                        .inner
                        .econ
                        .items
                        .get(item)
                        .and_then(|i| w.inner.econ.tables.item(i.record))
                        .map_or(0, |r| u32::from(r.quest));
                    if quest != 0 && w.quest_chain(item).is_none_or(|c| c.0.is_empty()) {
                        q.add_link(w, item, (quest - 1) as u8, None);
                    }
                    q.dispatch_chain(
                        w,
                        d2_sim::world::quests::event::ITEM_PICKED_UP,
                        d2_sim::world::quests::EventArgs {
                            player: Some(player),
                            target: Some(item),
                            ..Default::default()
                        },
                        false,
                    );
                }
            });
        });
        // Sent from inside the pick-up: ahead of the call's own messages
        // (the ground item's 0x0A), as the game's input pass sends them.
        self.collect_sent(events);
    }

    /// Runs the queued quest events and the level changes since the last
    /// tick on the quest control.
    pub(super) fn run_quest_events<D: ActionEvents>(&mut self, game: &mut Game, events: &mut D) {
        let mut queued = events.action().sys.hooks.x.take_quest_events();
        // The cube's `hst ` hook, recorded by its pending.
        if let Some(cube) = self.cube.as_mut() {
            for (player, code) in cube.pending.take_quest_items() {
                if code == *b"hst " {
                    queued.push(QuestEvent::StaffAssembled { player });
                }
            }
        }
        // The Golden Bird's boss choice reads the
        // monstats flags byte; only its bit 6 (`flying`, flag word
        // bit 14) is tested, so the byte is
        // built from that column. `None`: no monstats row.
        let bosses: Vec<(d2_sim::units::UnitId, u16, Option<u8>)> = {
            let sys = &events.action().sys;
            queued
                .iter()
                .filter_map(|e| match *e {
                    QuestEvent::BossCreated { unit } => Some(unit),
                    _ => None,
                })
                .filter_map(|u| {
                    let class = sys.units.get(u)?.class;
                    let row = sys.hooks.tables.combat.monstats.get(class as usize);
                    let flags = row.map(|m| if m.flying { act3::q4::FLYING_0D } else { 0 });
                    Some((u, u16::try_from(class).ok()?, flags))
                })
                .collect()
        };
        // The preset-boss event does nothing for units in levels ≥ 108; below
        // that level it runs the council's preset handler.
        let council: Vec<d2_sim::units::UnitId> = {
            let a = events.action();
            queued
                .iter()
                .filter_map(|e| match *e {
                    QuestEvent::PresetBoss { unit } => Some(unit),
                    _ => None,
                })
                .filter(|&u| {
                    let room = game.lists.unit(u).and_then(|e| e.room());
                    room.and_then(|r| a.sys.hooks.drlg.level_id(game, r))
                        .is_some_and(|l| l < 108)
                })
                .collect()
        };
        // Tick step 8's period: 20 base ticks (800 ms) at the game's rate.
        let quests_due = d2_sim::tick::is_due(game.frame, game.ticks(d2_sim::tick::period::QUESTS));
        let mut durance = None;
        let mut act3_npcs = None;
        let mut lair = None;
        let mut summit = None;
        let mut armed = false;
        let mut not_intro = Vec::new();
        let mut rescue = Vec::new();
        let mut quest_flags = Vec::new();
        self.desk(game, events, |desk, ctl, inv| {
            let ((), _) = quest_call(desk, ctl, inv, |q, w| {
                link_items(q, w);
                for e in &queued {
                    if let QuestEvent::Link { unit, chain } = *e {
                        q.add_link(w, unit, chain, None);
                    }
                }
                for e in &queued {
                    match *e {
                        QuestEvent::RadamentActivated { unit } => act2::q1::radament_ai(q, w, unit),
                        QuestEvent::SummonerActivated => act2::q5::summoner_seen(q, w),
                        QuestEvent::StaffAssembled { player } => {
                            act2::q2::staff_assembled(q, w, player)
                        }
                        QuestEvent::ShenkActivated { unit } => {
                            act5::q1::shenk_activated(q, w, unit)
                        }
                        QuestEvent::NihlathakActivated => act5::q4::nihlathak_ai_status(q, w),
                        QuestEvent::AncientsDisarm => act5::q5::disarm(q),
                        QuestEvent::BaalToStairs => act5::q6::chamber_open(q, w),
                        QuestEvent::AnyaOpenPortal { unit } => act5::q4::anya_ai_portal(q, w, unit),
                        // The prisoner AI's hooks.
                        QuestEvent::WussieLeft { guid } => act5::q2::group_count_guid(q, guid),
                        QuestEvent::WussieRescue { player, unit } => {
                            act5::q2::rescue(q, w, player, unit)
                        }
                        QuestEvent::WussieWait => act5::q2::rescue_status(q, w),
                        // Tyrael's spawn from the baalfx missile,
                        // at the missile's position of the call.
                        QuestEvent::SpawnTyrael {
                            room: Some(room),
                            x,
                            y,
                            ..
                        } => {
                            act5::q6::spawn_tyrael_at(w, room, x, y);
                        }
                        QuestEvent::AlkorReset => act3::alkor_bird_clear(q),
                        QuestEvent::OrmusAltar => act3::activate_altar(q, w),
                        // C→S 0x44: the staff in the orifice.
                        QuestEvent::InsertItem {
                            player,
                            object,
                            item,
                            action,
                        } => {
                            let item = w.unit_by_guid(UnitType::Item as u8, item);
                            act2::q6::item_to_object(q, w, player, object, item, action)
                        }
                        _ => {}
                    }
                }
                for &(unit, class, flags) in &bosses {
                    act3::choose_bird_boss(q, w, unit, class, flags);
                }
                for &unit in &council {
                    act3::council_preset(q, w, unit);
                }
                for e in &queued {
                    if let QuestEvent::Kill { victim, killer } = *e {
                        monster_killed(q, w, victim, killer);
                    }
                }
                // Tick step 8: the quest updater (timers such as A1Q2's 15)
                // runs on its period (`quests_due`).
                if quests_due {
                    q.update(w);
                }
                lair = act2::q6::lair_warp_open(q);
                // From any level but Durance 2 (the host tests the source).
                durance = Some(act3::durance_open(q, 0));
                act3_npcs = Some((act3::alkor_bird_brought(q), act3::altar_position(q)));
                // The exits are closed while the record's not-intro is set and
                // the Ancients are not defeated.
                summit = Some(act5::q5::summit_warp_open(q));
                armed = act5::q5::armed(q);
                not_intro = q.records.iter().map(|r| (r.chain, r.not_intro)).collect();
                rescue = act5::q2::barbarian_states(q);
                let d = usize::from(w.difficulty());
                for p in w.players() {
                    if let Some(f) = w.quests(p).map(|r| r.flags[d]) {
                        quest_flags.push((p, f));
                    }
                }
            });
        });
        if let Some(open) = lair {
            events.action().sys.hooks.x.set_lair_open(open);
        }
        if let Some((bird, altar)) = act3_npcs {
            events
                .action()
                .sys
                .hooks
                .x
                .set_act3_npc_answers(bird, altar);
        }
        if let Some(open) = durance {
            events.action().sys.hooks.x.set_durance_open(open);
        }
        events.action().sys.hooks.x.set_ancients_armed(armed);
        events
            .action()
            .sys
            .hooks
            .x
            .publish_quest_flags(&quest_flags);
        if let Some(open) = summit {
            events.action().sys.hooks.x.set_summit_open(open);
        }
        if !not_intro.is_empty() {
            events.action().sys.hooks.x.publish_not_intro(&not_intro);
            events.action().sys.hooks.x.publish_rescue(&rescue);
        }
    }
}

/// One queued kill: the kill parse, which adds no
/// links; a boss's chains are linked at its creation.
fn monster_killed<W: QuestWorld>(
    q: &mut d2_sim::world::quests::QuestControl,
    w: &mut W,
    victim: d2_sim::units::UnitId,
    killer: Option<d2_sim::units::UnitId>,
) {
    q.monster_killed(w, victim, killer);
}
