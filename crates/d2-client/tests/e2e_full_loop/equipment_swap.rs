//! BUG-029: real inventory moves, emitted item packets, client cursor and walking.
use super::*;
use d2_client::bridge::{click, items};
use d2_sim::items::inventory::{grid_id, BODY_GRID};

fn item_message(id: u8, item: u32, loc: Option<u32>) -> Vec<u8> {
    let mut bytes = vec![id];
    bytes.extend(item.to_le_bytes());
    if let Some(loc) = loc {
        bytes.extend(loc.to_le_bytes());
    }
    bytes
}

fn join_client(fx: &mut Fx) {
    let guid = fx.guid(fx.player);
    let (x, y) = fx.pos(fx.player);
    let mut assign = vec![0x59];
    assign.extend(guid.to_le_bytes());
    assign.push(0);
    assign.resize(22, 0);
    assign.extend((x as u16).to_le_bytes());
    assign.extend((y as u16).to_le_bytes());
    let mut handshake = vec![0x0B, 0];
    handshake.extend(guid.to_le_bytes());
    fx.bridge.link_mut().inject.extend([assign, handshake]);
    assert!(!fx.bridge.frame().unwrap().ticked);
    let player = fx.player;
    let sim = fx.sim();
    sim.announce_ground = true;
    sim.events
        .action
        .with(&mut sim.game, |_, v| v.set_base(player, 0, 25));
    fx.inv.with(|state| state.mouse_skill = Some((0, -1)));
    for item in fx.inventory() {
        announce_stored_item(fx, item);
    }
    fx.step(&[]);
}

fn announce_stored_item(fx: &mut Fx, item: UnitId) {
    use d2_sim::items::moves::{deferred, MoveUnits};
    let owner = Owner::player(fx.guid(fx.player));
    let guid = fx.guid(item);
    let sim = fx.sim();
    sim.world
        .with_economy(&mut sim.game, &mut sim.events, |econ, parts| {
            let mut desk = parts.inventory.as_deref_mut().unwrap().desk(econ);
            let flags = MoveUnits::item_flags(&desk, guid);
            MoveUnits::set_item_flags(&mut desk, guid, flags | flag::IDENTIFIED);
            deferred::mark(
                &mut desk,
                owner,
                guid,
                d2_sim::items::moves::cmd::PUT_IN_PAGE,
            );
        });
}

#[track_caller]
fn cursor_agrees(fx: &Fx, expected: Option<u32>) {
    let inv = &fx
        .sim_ref()
        .world
        .inventory
        .as_ref()
        .unwrap()
        .state
        .inventories[&fx.player];
    let server = inv.cursor().map(|unit| fx.guid(unit));
    let client = items::cursor_item(fx.bridge.world()).map(|item| item.key.guid);
    assert_eq!(server, expected);
    assert_eq!(client, server, "rejected: {:?}", fx.bridge.log().rejected);
}

fn can_walk(fx: &mut Fx) {
    let local = &fx.bridge.world().units[&fx.bridge.world().local_player.unwrap()];
    assert!(click::can_act(
        true,
        false,
        local.mode,
        local.class,
        None,
        0
    ));
    let player = fx.player;
    let before = fx.pos(player);
    fx.stage_facts();
    fx.step(&[bytes(&Walk {
        x: (before.0 + 3) as u16,
        y: before.1 as u16,
    })]);
    for _ in 0..4 {
        fx.step(&[]);
    }
    assert_ne!(fx.pos(player), before);
    assert!(fx.errors().is_empty(), "{:?}", fx.errors());
}

#[test]
fn belt_evacuation_keeps_the_displaced_item_on_both_cursors() {
    let mut fx = Fx::new(GAME_SEED);
    join_client(&mut fx);
    let inventory = fx.sim().world.inventory.as_mut().unwrap();
    for record in [CAP, BUC] {
        inventory.tables.items[record].type_ = 19;
        inventory.tables.items[record].invwidth = 1;
        inventory.tables.items[record].invheight = 1;
    }
    inventory.tables.itemtypes[19].bodyloc1 = 8;
    inventory.tables.itemtypes[19].bodyloc2 = 8;
    inventory.tables.itemtypes[19].body = 1;
    inventory.tables.itemtypes[19].beltable = 1;
    inventory.tables.belts.resize(3, 4);
    inventory.tables.belts[0] = 16;
    inventory.tables.belts[1] = 4;
    inventory.tables.items[CAP].belt = 0;
    inventory.tables.items[BUC].belt = 1;
    let player = fx.player;
    let sim = fx.sim();
    // Another one-cell item represents a potion in a belt slot that is lost
    // when replacing the sixteen-slot belt with a four-slot belt.
    let content = sim
        .world
        .with_economy(&mut sim.game, &mut sim.events, |econ, _| {
            let mut request = ItemRequest {
                item: CAP as i32,
                ilvl: 1,
                quality: 2,
                format: 1,
                ..ItemRequest::default()
            };
            econ.create_item(
                &mut request,
                false,
                ItemSpawn {
                    room: None,
                    mode: 4,
                    init_flags: 1,
                },
            )
            .unwrap()
        });
    store(
        &mut sim.world,
        &mut sim.game,
        &mut sim.events,
        (player, content),
        0,
    );
    announce_stored_item(&mut fx, content);
    fx.step(&[]);
    let content = fx.guid(content);
    let old = fx.guid(fx.cap);
    let incoming = fx.guid(fx.buckler);
    fx.step(&[item_message(0x19, old, None)]);
    fx.step(&[item_message(0x1A, old, Some(8))]);
    fx.step(&[item_message(0x19, content, None)]);
    fx.step(&[item_message(0x23, content, Some(8))]);
    cursor_agrees(&fx, None);
    fx.step(&[item_message(0x19, incoming, None)]);
    fx.step(&[item_message(0x1D, incoming, Some(8))]);
    cursor_agrees(&fx, Some(old));
    fx.step(&[item_message(0x17, old, None)]);
    cursor_agrees(&fx, None);
    can_walk(&mut fx);
}

#[test]
fn successful_and_failed_equipment_swaps_keep_cursor_agreement_and_allow_walking() {
    for (opcode, loc) in [(0x1B, 4u32), (0x1D, 1), (0x1D, 4), (0x1E, 4)] {
        for fail_placement in [false, true] {
            let mut fx = Fx::new(GAME_SEED);
            join_client(&mut fx);
            let inventory = fx.sim().world.inventory.as_mut().unwrap();
            for record in [CAP, BUC] {
                let ty = inventory.tables.items[record].type_ as usize;
                inventory.tables.itemtypes[ty].bodyloc1 = loc as u8;
                inventory.tables.itemtypes[ty].bodyloc2 = if loc == 1 { 1 } else { 5 };
            }
            // The cap stands for a weapon; the buckler stands for the other
            // equippable family. Synthetic equivalence allows a legal hand pair.
            let ty = inventory.tables.items[CAP].type_ as usize;
            let weapon = d2_sim::items::inventory::ty::WEAP as usize;
            let words = inventory.tables.equiv.words;
            inventory.tables.equiv.bits[ty * words + weapon / 32] |= 1 << (weapon % 32);
            let cap = fx.guid(fx.cap);
            let buckler = fx.guid(fx.buckler);
            let (old, incoming) = if opcode == 0x1E {
                let player = fx.player;
                let sim = fx.sim();
                let extra = sim
                    .world
                    .with_economy(&mut sim.game, &mut sim.events, |econ, _| {
                        let mut request = ItemRequest {
                            item: CAP as i32,
                            ilvl: 1,
                            quality: 2,
                            format: 1,
                            ..ItemRequest::default()
                        };
                        econ.create_item(
                            &mut request,
                            false,
                            ItemSpawn {
                                room: None,
                                mode: 4,
                                init_flags: 1,
                            },
                        )
                        .unwrap()
                    });
                store(
                    &mut sim.world,
                    &mut sim.game,
                    &mut sim.events,
                    (player, extra),
                    0,
                );
                announce_stored_item(&mut fx, extra);
                fx.step(&[]);
                fx.step(&[item_message(0x19, buckler, None)]);
                fx.step(&[item_message(0x1A, buckler, Some(4))]);
                fx.step(&[item_message(0x19, cap, None)]);
                fx.step(&[item_message(0x1A, cap, Some(5))]);
                fx.sim().world.inventory.as_mut().unwrap().tables.items[CAP].twohanded = 1;
                (buckler, fx.guid(extra))
            } else {
                fx.step(&[item_message(0x19, cap, None)]);
                cursor_agrees(&fx, Some(cap));
                fx.step(&[item_message(
                    0x1A,
                    cap,
                    Some(if opcode == 0x1B { 5 } else { loc }),
                )]);
                if opcode == 0x1B {
                    fx.sim().world.inventory.as_mut().unwrap().tables.items[BUC].twohanded = 1;
                }
                (cap, buckler)
            };
            cursor_agrees(&fx, None);
            fx.step(&[item_message(0x19, incoming, None)]);
            cursor_agrees(&fx, Some(incoming));
            if fail_placement {
                let player = fx.player;
                // The old cell remains readable; placement rejects the dimensions
                // only after the swap has removed the displaced equipment.
                fx.sim()
                    .world
                    .inventory
                    .as_mut()
                    .unwrap()
                    .state
                    .inventories
                    .get_mut(&player)
                    .unwrap()
                    .grid_or_create(grid_id::BODY, BODY_GRID.0, BODY_GRID.1)
                    .unwrap()
                    .width += 1;
            }
            let result = fx.step(&[item_message(opcode, incoming, Some(loc))]);
            assert_eq!(result.codes.len(), 1);
            assert_eq!(result.codes[0].0, opcode);
            let held = if fail_placement { incoming } else { old };
            cursor_agrees(&fx, Some(held));
            let player = fx.player;
            if fail_placement {
                fx.sim()
                    .world
                    .inventory
                    .as_mut()
                    .unwrap()
                    .state
                    .inventories
                    .get_mut(&player)
                    .unwrap()
                    .grid_or_create(grid_id::BODY, BODY_GRID.0 + 1, BODY_GRID.1)
                    .unwrap()
                    .width = BODY_GRID.0;
            }
            assert!(fx
                .sim_ref()
                .game
                .lists
                .find_unit(UnitType::Item, old)
                .is_some());
            assert!(fx
                .sim_ref()
                .game
                .lists
                .find_unit(UnitType::Item, incoming)
                .is_some());
            // A held item is normally placed or dropped before a world click walks.
            let client_held = items::cursor_item(fx.bridge.world()).unwrap().key.guid;
            fx.step(&[item_message(0x17, client_held, None)]);
            cursor_agrees(&fx, None);
            can_walk(&mut fx);
        }
    }
}

#[test]
fn exhausted_throw_stack_is_removed_without_a_phantom_cursor() {
    let mut fx = Fx::new(GAME_SEED);
    join_client(&mut fx);
    let inventory = fx.sim().world.inventory.as_mut().unwrap();
    let ty = inventory.tables.items[CAP].type_ as usize;
    inventory.tables.itemtypes[ty].bodyloc1 = 4;
    inventory.tables.itemtypes[ty].bodyloc2 = 5;
    let thrown = fx.guid(fx.cap);
    let held = fx.guid(fx.buckler);
    fx.step(&[item_message(0x19, thrown, None)]);
    fx.step(&[item_message(0x1A, thrown, Some(4))]);
    fx.step(&[item_message(0x19, held, None)]);
    let item = fx.cap;
    let sim = fx.sim();
    sim.events
        .action
        .with(&mut sim.game, |_, v| v.set_base(item, 70, 0));
    let mut mark = vec![d2_sim::wiring::interaction::skill_use::EXHAUST_STACK_MARK];
    mark.extend(thrown.to_le_bytes());
    let player = fx.player;
    fx.pending().sent.push((player, mark));
    fx.step(&[]);
    fx.step(&[]);
    cursor_agrees(&fx, Some(held));
    assert_eq!(
        fx.sim_ref()
            .world
            .inventory
            .as_ref()
            .unwrap()
            .state
            .inventories[&player]
            .body_item(4),
        None
    );
    assert!(d2_client::bridge::items::items(fx.bridge.world())
        .iter()
        .all(|item| item.key.guid != thrown || item.mode != d2_client::bridge::items::mode::BODY));
    assert!(fx
        .sim_ref()
        .game
        .lists
        .find_unit(UnitType::Item, thrown)
        .is_none());
    fx.step(&[item_message(0x17, held, None)]);
    cursor_agrees(&fx, None);
    can_walk(&mut fx);
}
