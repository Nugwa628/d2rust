//! One synthetic test per rule unit not covered by `tests.rs` /
//! `local_tests.rs`: only the bridge talks to the link, any
//! link serves the bridge unchanged, the world model's fields,
//! no interpolation or prediction, the dispatch
//! TSV is source, and nothing bridge-side is written to disk.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use d2_proto::client::Walk;
use d2_proto::PROTOCOL_VERSION;

use super::dispatch::{self, Dispatch, HandlerError, Message};
use super::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use super::world::{ClientUnit, ClientWorld, UnitKey};
use super::Bridge;

/// A link that is neither the local host nor a test of `tests.rs`: it
/// stands for "another `ServerLink`" (an online link).
#[derive(Default)]
struct OtherLink {
    sent: Vec<(SendQueue, Vec<u8>)>,
    pumps: usize,
    deliveries: VecDeque<(bool, Vec<Vec<u8>>)>,
    current: Vec<Vec<u8>>,
}

impl ServerLink for OtherLink {
    fn protocol_version(&self) -> u32 {
        PROTOCOL_VERSION
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.sent.push((queue, msg.to_vec()));
        Ok(Sent::Queued)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.pumps += 1;
        let (ticked, chunks) = self.deliveries.pop_front().unwrap_or_default();
        self.current = chunks;
        Ok(Pumped { ticked })
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.current)
    }
}

/// A synthetic handler: inserts the addressed unit (mechanism only).
fn insert_unit(world: &mut ClientWorld, m: &Message<'_>) -> Result<(), HandlerError> {
    if let Some(key) = m.unit {
        world.units.insert(key, ClientUnit::new(key));
    }
    Ok(())
}

fn unit_dispatch() -> Dispatch {
    let mut d = Dispatch::empty();
    d.set(0x6D, "test", insert_unit);
    d
}

/// S→C 0x6D (10 bytes): a monster unit message, GUID at +1.
fn monster_msg(guid: u32) -> Vec<u8> {
    let mut m = vec![0u8; 10];
    m[0] = 0x6D;
    m[1..5].copy_from_slice(&guid.to_le_bytes());
    m
}

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every non-test `.rs` file under `dir`, with its code lines (outside
/// `//` comments), in path order.
fn sources(dir: &Path) -> Vec<(PathBuf, Vec<String>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if !name.ends_with(".rs") || name.contains("tests") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            let lines = text
                .lines()
                .map(|l| l.split("//").next().unwrap_or("").to_string())
                .filter(|l| !l.trim().is_empty())
                .collect();
            out.push((p, lines));
        }
    }
    out.sort();
    out
}

#[test]
fn only_the_bridge_talks_to_the_server_link() {
    let src = src_dir();
    let bridge = src.join("bridge");
    let mut callers = Vec::new();
    for (path, lines) in sources(&src) {
        if path.starts_with(&bridge) {
            continue;
        }
        // A link adapter (a `ServerLink` that wraps another link, e.g. the
        // server thread) forwards the link calls; it is part of the link.
        let is_link = lines
            .iter()
            .any(|l| l.contains("impl") && l.contains("ServerLink for"));
        for l in &lines {
            if l.contains(".pump()") || l.contains(".receive()") || l.contains("link_mut(") {
                callers.push((path.clone(), is_link, l.trim().to_string()));
            }
        }
    }
    let outside: Vec<_> = callers.iter().filter(|c| !c.1).collect();
    assert!(
        outside.is_empty(),
        "link calls outside the bridge: {outside:?}"
    );
    // The scan sees the adapter's forwarding calls (it can fail).
    assert!(callers.iter().any(|c| c.1), "{callers:?}");
    // Bevy reaches the game through the resource: the mirror's systems
    // hold `BridgeResource` and call the bridge, never a link.
    let mirror = std::fs::read_to_string(bridge.join("mirror.rs")).unwrap();
    assert!(mirror.contains(".display_frame(drawn.is_none())"));
    assert!(!mirror.contains(".pump()") && !mirror.contains(".receive()"));
}

#[test]
fn any_server_link_serves_the_bridge_unchanged() {
    // Only the local adapter names the in-process server; the bridge
    // itself is written against the trait.
    let bridge = src_dir().join("bridge");
    for (path, lines) in sources(&bridge) {
        let local = path.ends_with("local.rs");
        let names_server = lines.iter().any(|l| l.contains("d2_server"));
        assert_eq!(names_server, local, "{}", path.display());
    }
    // Another link, boxed as the app holds it, runs the same send and
    // receive paths.
    let mut link = OtherLink::default();
    link.deliver(true, vec![monster_msg(7)]);
    let mut b =
        Bridge::with_dispatch(Box::new(link) as Box<dyn ServerLink>, unit_dispatch()).unwrap();
    b.send(&Walk { x: 3, y: 4 }).unwrap();
    let report = b.frame().unwrap();
    assert_eq!((report.ticked, report.handled), (true, 1));
    assert!(b.world().units.contains_key(&UnitKey {
        unit_type: 1,
        guid: 7
    }));
}

impl OtherLink {
    fn deliver(&mut self, ticked: bool, chunks: Vec<Vec<u8>>) {
        self.deliveries.push_back((ticked, chunks));
    }
}

#[test]
fn client_world_holds_only_stated_fields() {
    // Exhaustive destructuring: a new field fails to compile here until
    // its owner states it and this test names it.
    // The fields of the client model (step 1) and the bridge (step 3).
    let ClientWorld {
        frames,
        // D2RUST's own, unverified: the shop panel's trade epoch.
        store_serial,
        state_on_seq,
        state_on_next,
        // The decoder's tables handle.
        item_tables: _,
        // Charstats copies for the full array.
        class_vitals: _,
        // itemstatcost op tables for the percent ops.
        stat_ops: _,
        tip_anims: _,
        // D2RUST's own, unverified: session messages for the loading flow
        // (L9, L10).
        session_total,
        session_log,
        server_ticks,
        // The game's rate, supplied by the link.
        tick_rate,
        server_frame,
        // 0x15 flags of the local player.
        local_places: _,
        // The automap countdown's placement count.
        placed_units: _,
        units,
        local_player,
        // D2RUST bookkeeping: the play preview's predicted position (B025).
        local_predicted: _,
        // D2RUST bookkeeping: faces to redo with the fresh prediction.
        local_faces: _,
        // The interact NPC of the last dialog.
        interact_npc: _,
        difficulty,
        expansion,
        ladder,
        game_flags,
        act,
        // The 0x03 loads handled.
        act_loads: _,
        in_game,
        unloaded,
        exit_requested,
        rooms_in_sight,
        outgoing,
        // D2RUST's own, unverified: the play preview's walk cell for the
        // position check.
        local_walk,
        // D2RUST's own, unverified: the turn rate of the local path direction.
        local_turn: _,
        use_cursor,
        // and 4,.
        pets,
        palette_act,
        drlg,
        active_rooms,
        // (last paragraph).
        room_units,
        lights,
        // The stat list's cast light.
        cast_lights,
        // function 2: the drawn frame's unit
        // origin.
        unit_origin,
        // The running screen shake.
        shake,
        drlg_updates,
        // The skill-use lock.
        skill_lock,
        room_allied,
        environment,
        eclipse_pending,
        skill_tree_flag,
        env_period_cache,
        overrides,
        quest_availability,
        // The records the sound layer's
        // level-entry check reads.
        quest_player,
        quest_game,
        quest_status,
        quest_counters,
        rooms_loaded,
        roster,
        roster_inactive,
        weapon_set,
        item_table_ext,
        // (pet pass).
        connected,
        ping,
        pet_palette,
        belt_ready,
        // (set C);
        // (the latches).
        objclient,
        // Frees not yet moved out as
        // `UnitFreed` outputs.
        freed,
        // D2RUST's own, unverified: scripted clicks run after the update.
        late_input,
        // The tables of the corpse test.
        corpse_tables,
        // The lit overlay records.
        overlay_lights,
        // The walk's update mark.
        overlay_stamp: _,
        // The hit overlay creates for the effect layer.
        hit_overlays,
        // Recorded graphics-ready
        // flags of a replayed capture.
        gfx_ready,
        // The classes
        // an S→C 0xAC loaded the palshift block of.
        palshift_classes,
        // The last authority view the client acknowledged.
        view_ready,
    } = ClientWorld::default();
    assert!(gfx_ready.is_none());
    assert!(view_ready.is_none());
    assert!(palshift_classes.is_empty());
    assert!(freed.is_empty());
    assert!(!late_input);
    assert!(hit_overlays.is_empty());
    assert_eq!(corpse_tables, Default::default());
    assert!(!connected && ping == Default::default() && pet_palette.is_empty());
    assert_eq!(belt_ready, [false; 4]);
    assert!(local_walk.is_none());
    assert_eq!(objclient, Default::default());
    assert!(overrides == Default::default() && roster.is_empty() && roster_inactive.is_empty());
    assert!(quest_availability.is_none() && rooms_loaded.is_empty());
    assert!(quest_player.is_none() && quest_game.is_none() && quest_status.is_none());
    assert_eq!(quest_counters, [0; 3]);
    assert!(weapon_set == 0 && item_table_ext.is_empty());
    assert!(pets.is_empty() && palette_act.is_none() && active_rooms.is_none());
    assert!(
        room_units == Default::default()
            && lights.is_empty()
            && drlg_updates == 0
            && skill_lock == 0
    );
    assert!(cast_lights.is_empty() && overlay_lights.is_empty());
    assert!(drlg.is_none() && unit_origin.is_none() && shake.is_none());
    assert!(environment.is_none() && !eclipse_pending && skill_tree_flag.is_none());
    assert_eq!(env_period_cache, 0);
    assert!(room_allied.is_empty());
    assert_eq!((frames, server_ticks, units.len()), (0, 0, 0));
    assert_eq!(tick_rate, d2_sim::tick::TickRate::BASE);
    assert_eq!(server_frame, None);
    assert_eq!(store_serial, 0);
    assert!(state_on_seq.is_empty());
    assert_eq!(state_on_next, 0);
    assert!(session_total == 0 && session_log.is_empty());
    assert_eq!((local_player, act, use_cursor), (None, None, None));
    assert_eq!((difficulty, expansion, ladder, game_flags), (0, 0, 0, 0));
    assert!(!in_game && !unloaded && !exit_requested);
    assert!(rooms_in_sight.is_empty() && outgoing.is_empty());
    let key = UnitKey {
        unit_type: 1,
        guid: 2,
    };
    let ClientUnit {
        key: k,
        class,
        mode,
        position,
        server_point,
        stats,
        seed,
        queue,
        last_mode_request,
        request_late,
        // D2RUST bookkeeping of step 1 (no base-game field).
        mode_requests,
        last_cast,
        last_path_request,
        placements: _,
        anim_restarts: _,
        mode_changes: _,
        path_dir: _,
        path_turn: _,
        copy_turned: _,
        // D2RUST bookkeeping: the turns counted for the prediction (B025).
        faces: _,
        cast_target: _,
        // The used skill and the
        // monster's sequence state.
        used_skill: _,
        used_skill_level: _,
        sequence: _,
        // D2RUST's own, unverified: the player path's target point and the unit
        // it follows.
        path_target: _,
        path_follow: _,
        path_live: _,
        leap_arc: _,
        // D2RUST bookkeeping of the play preview's rule-8 follow.
        follows,
        kind,
        skills,
        quest_untargetable,
        flag_2,
        states,
        // (+0x6C).
        palette_index,
        state_lists,
        timed_lists,
        // (`cltstfunc` 9);.
        temp_lists,
        // (open question 10 decided as A).
        turned_toward,
        path_stopped,
        direction_of,
        room_freed,
        flag_200,
        // (+0xC4 bit 0x100000).
        flag_flat,
        // (+0xC4 bit 0x20, no shadow).
        flag_20,
        interact_ms,
        frame,
        // The client track's precise position (`bridge::motion`).
        precise,
        // +0x48, a monster's mode set.
        frame_count,
        flag_ex,
        // (item update: overlay 70 held).
        ground_overlay,
        ground_overlay_seed,
        flag_4,
        // (+0xB0).
        hit_class,
        // (+0x4C).
        speed,
        // (+0x38 >> 8).
        event_index,
        // (+0xC4 bit 0x40, +0x4E, AnimData).
        flag_40,
        action_frame,
        anim_events,
        last_hit_overlay,
        hit_overlays,
        // D2RUST bookkeeping: the update's own mode ends.
        mode_ends,
        // (+0x58, the hit flash).
        hit_flash,
        hit_flash_interact,
        // D2RUST bookkeeping: seed sets.
        seed_sets,
    } = ClientUnit::new(key);
    assert_eq!((hit_flash, hit_flash_interact), (0, false));
    assert_eq!((flag_40, action_frame, anim_events), (false, 0, None));
    assert_eq!(mode_ends, 0);
    assert_eq!((interact_ms, frame, flag_ex, flag_4), (0, 0, 0, false));
    assert_eq!(frame_count, 0);
    assert!(ground_overlay.is_none() && ground_overlay_seed.is_none());
    assert_eq!(precise, None);
    assert_eq!(speed, None);
    assert_eq!((hit_class, event_index), (0, 0));
    assert!(skills.is_none() && !quest_untargetable);
    assert!(turned_toward.is_none() && !path_stopped);
    assert!(direction_of.is_none() && !room_freed && !flag_200 && !flag_flat && !flag_20);
    assert!(flag_2.is_none() && states.is_empty() && state_lists.is_empty());
    assert_eq!(palette_index, 0);
    assert!(timed_lists.is_empty() && temp_lists.is_empty());
    assert_eq!(k, key);
    assert_eq!((class, mode, position, server_point), (0, 0, None, (0, 0)));
    assert_eq!((seed, seed_sets), (Some((1, 666)), 0));
    assert!(stats.is_empty() && queue.is_empty() && last_mode_request.is_none() && !request_late);
    assert!(last_cast.is_none());
    assert!(last_hit_overlay.is_none() && hit_overlays == 0);
    assert!(last_path_request.is_none());
    assert_eq!((mode_requests, follows), (0, 0));
    assert_eq!(kind, super::world::KindData::None);
    // The model shares the tick-rate value type, but holds no simulation state.
    let world = std::fs::read_to_string(src_dir().join("bridge/world.rs")).unwrap();
    assert!(!world
        .replace("d2_sim::tick::TickRate", "")
        .contains("d2_sim"));
}

#[test]
fn no_interpolation_or_prediction_between_messages() {
    let mut link = OtherLink::default();
    link.deliver(true, vec![monster_msg(7)]);
    for ticked in [false, true, false, true] {
        link.deliver(ticked, vec![]);
    }
    let mut b = Bridge::with_dispatch(link, unit_dispatch()).unwrap();
    b.frame().unwrap();
    let stated = b.world().units.clone();
    // Frames without S→C messages (ticked or not) and sent intents leave
    // every unit as the last message stated it: nothing advances it.
    for _ in 0..4 {
        b.send(&Walk { x: 9, y: 9 }).unwrap();
        b.frame().unwrap();
        assert_eq!(b.world().units, stated);
    }
    assert_eq!((b.world().frames, b.world().server_ticks), (5, 3));
}

#[test]
fn dispatch_tsv_is_source_compiled_in() {
    // The table is the TSV file, built into the binary: equal to it byte
    // for byte, read with no file I/O and no version field.
    let on_disk = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tables/client/bridge-dispatch.tsv"),
    )
    .unwrap();
    assert_eq!(dispatch::TSV, on_disk);
    assert_eq!(dispatch::TSV.lines().next(), Some("id\tname\towner"));
    let rows = dispatch::parse(dispatch::TSV).unwrap();
    assert_eq!(rows.len(), dispatch::IDS);
    // The dispatch module reads no file at run time.
    let src = std::fs::read_to_string(src_dir().join("bridge/dispatch.rs")).unwrap();
    assert!(!src.contains("std::fs") && !src.contains("File::"));
}

#[test]
fn bridge_writes_nothing_to_disk() {
    // No bridge data is written yet, so no format (and no version field)
    // exists. A writer added here must bring its versioned format (step
    // 3); this scan fails until it is reviewed.
    for (path, lines) in sources(&src_dir().join("bridge")) {
        for l in &lines {
            for needle in ["std::fs", "File::", "OpenOptions", "fs::write", "BufWriter"] {
                assert!(!l.contains(needle), "{}: {l}", path.display());
            }
        }
    }
}
