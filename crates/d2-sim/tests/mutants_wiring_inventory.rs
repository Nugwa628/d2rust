//! Tests written against the surviving mutants of
//! `crates/d2-sim/src/wiring/inventory/` (`cargo mutants`; record in
//! `docs/handoff/mutants-wiring-inventory.md`). Two kinds:
//!
//! - **Seam forwarding** (`forward_*`): the adapters that only hand a
//!   call to the seams without a d2-sim provider (`InvRest` with its
//!   `MovePending` part) must pass every argument through and return the
//!   rest's answer unchanged.
//!   These checks isolate forwarding from the inventory implementation.
//! - **Inventory outcomes** through `items::moves::handle` / `player_update`
//!   or the inventory-model calls, checking state changes and messages.

#[path = "mutants_wiring_inventory/fixture.rs"]
mod fixture;

use d2_sim::items::inventory::{InteractionTarget, InvWorld, UnitKind};
use d2_sim::items::moves::{InventoryOps, MovePending, MoveUnits, Owner, Spot};
use d2_sim::units::hooks::MonsterInfo;
use d2_sim::units::{UnitId, UnitType};

use fixture::*;

/// Non-default answers, `flag` as given (each boolean mutant needs both).
fn wild(flag: bool, w: &World) -> Answers {
    Answers {
        distance: 77,
        collides: flag,
        room_at: flag,
        spot: Some(Spot {
            room: w.room,
            x: 31,
            y: 32,
        }),
        in_town: flag,
        flag,
        owners: vec![Owner::monster(901)],
        copy: Some(902),
        owner: Some(Owner::monster(903)),
        share_id: 904,
        code: 905,
        bits: vec![9, 0, 6],
        msgs: vec![vec![9, 0, 7]],
        two_handed: Default::default(),
        gold: false,
        number: 909,
        ammo: Some(910),
        trade_gate: Some(flag),
        open_ok: flag,
        code2: 912,
        level_req: 913,
    }
}

/// Every `MovePending` method of `pending.rs` that goes to the rest:
/// arguments and answer unchanged.
fn forward_pending(flag: bool) {
    let mut w = World::new();
    let a = wild(flag, &w);
    w.rest.a = a.clone();
    let me = w.me();
    let m = Owner::monster(55);
    w.desk(|d| {
        macro_rules! fwd {
            ($call:expr, $want:expr, $log:expr) => {{
                let got = $call;
                assert_eq!(got, $want, "{}", $log);
                assert_eq!(d.rest.last(), $log);
            }};
        }
        let p = format!("0:{}", me.guid);
        fwd!(d.distance(me, m), 77, format!("distance {p} 1:55"));
        fwd!(d.collides(me, m, 3), flag, format!("collides {p} 1:55 3"));
        fwd!(
            d.walk_to_item(me, 5, true),
            (),
            format!("walk_to_item {p} 5 true")
        );
        fwd!(d.room_at(6, 7), flag, "room_at 6 7".to_string());
        fwd!(
            d.free_spot((1, 2), (3, 4), 5, 6, 7, 8),
            a.spot,
            "free_spot (1, 2) (3, 4) 5 6 7 8".to_string()
        );
        fwd!(d.in_town(me), flag, format!("in_town {p}"));
        fwd!(
            d.room_delete_notice(9),
            (),
            "room_delete_notice 9".to_string()
        );
        fwd!(d.free_collision(10), (), "free_collision 10".to_string());
        fwd!(
            d.room_change_notice(11, 12, 13),
            (),
            "room_change_notice 11 12 13".to_string()
        );
        fwd!(
            MovePending::stat_refresh(d, m),
            (),
            "stat_refresh 1:55".to_string()
        );
        fwd!(
            d.stat_refresh_unlink(m, 14),
            (),
            "stat_refresh_unlink 1:55 14".to_string()
        );
        fwd!(
            MovePending::stat_link(d, m, 15),
            (),
            "stat_link 1:55 15".to_string()
        );
        fwd!(
            MovePending::charm_relink(d, m, 16),
            (),
            "charm_relink 1:55 16".to_string()
        );
        fwd!(
            d.charm_unlink(m, 17),
            (),
            "charm_unlink 1:55 17".to_string()
        );
        fwd!(
            MovePending::inventory_pass(d, m),
            (),
            "inventory_pass 1:55".to_string()
        );
        fwd!(
            MovePending::weapon_in_use_update(d, m),
            (),
            "weapon_in_use_update 1:55".to_string()
        );
        fwd!(
            MovePending::weapon_bookkeeping(d, m),
            (),
            "weapon_bookkeeping 1:55".to_string()
        );
        fwd!(
            d.body_leave_effects(m, 19),
            (),
            "body_leave_effects 1:55 19".to_string()
        );
        fwd!(
            d.hireling_owner_pass(m),
            (),
            "hireling_owner_pass 1:55".to_string()
        );
        fwd!(d.sound(m, 21), (), "sound 1:55 21".to_string());
        fwd!(
            d.pickup_sound(m, 22),
            (),
            "pickup_sound 1:55 22".to_string()
        );
        fwd!(
            d.requirement_sound(m),
            (),
            "requirement_sound 1:55".to_string()
        );
        fwd!(d.merc_sound(m), (), "merc_sound 1:55".to_string());
        fwd!(
            d.quest_flag(m, 23, 24),
            flag,
            "quest_flag 1:55 23 24".to_string()
        );
        fwd!(
            d.quest_item_picked(m, 25),
            (),
            "quest_item_picked 1:55 25".to_string()
        );
        fwd!(
            d.quest_item_dropped(26),
            (),
            "quest_item_dropped 26".to_string()
        );
        fwd!(d.carry_one(27), flag, "carry_one 27".to_string());
        fwd!(
            d.held_test_units(m),
            a.owners,
            "held_test_units 1:55".to_string()
        );
        // copy_item / give_cursor_item run on the desk's model (hireling take).
        fwd!(d.consume_one(30), flag, "consume_one 30".to_string());
        fwd!(d.set_owner(31, m), (), "set_owner 31 1:55".to_string());
        fwd!(d.pile_owner(32), a.owner, "pile_owner 32".to_string());
        fwd!(
            d.gold_drop_has_no_owner(),
            flag,
            "gold_drop_has_no_owner".to_string()
        );
        fwd!(d.party_share_id(m), 904, "party_share_id 1:55".to_string());
        fwd!(d.party_share(m, 33), (), "party_share 1:55 33".to_string());
        fwd!(
            d.owned_gold_pickup(m, 34, 35),
            (),
            "owned_gold_pickup 1:55 34 35".to_string()
        );
        fwd!(d.rest_pile(m, 36), (), "rest_pile 1:55 36".to_string());
        fwd!(
            d.book_count_changed(m, 0, 37),
            (),
            "book_count_changed 1:55 37".to_string()
        );
        fwd!(d.use_item(m, me, 41), flag, format!("use_item 1:55 {p} 41"));
        fwd!(
            d.charge_update(m, 42),
            (),
            "charge_update 1:55 42".to_string()
        );
        fwd!(d.remove_used(m, 43), (), "remove_used 1:55 43".to_string());
        fwd!(
            d.filler_linked(50, 51),
            (),
            "filler_linked 50 51".to_string()
        );
        // The runeword is the desk's (`queries::activate_runeword_on`):
        // no item 52, no runeword, the rest not asked.
        {
            let before = d.rest.last();
            assert!(!d.runeword(m, 52));
            assert_eq!(d.rest.last(), before, "runeword is not forwarded");
        }
        fwd!(d.hireling(m), a.owner, "hireling 1:55".to_string());
        fwd!(
            d.owns_hireling(me, m),
            flag,
            format!("owns_hireling {p} 1:55")
        );
        fwd!(
            d.equip_on_merc(m, 53),
            (),
            "equip_on_merc 1:55 53".to_string()
        );
        fwd!(d.merc_after_take(m), (), "merc_after_take 1:55".to_string());
        fwd!(d.pick_npc(m, 54, 1), 905, "pick_npc 1:55 54 1".to_string());
        fwd!(
            d.pick_object(m, 56, 1),
            905,
            "pick_object 1:55 56 1".to_string()
        );
        fwd!(d.send(m, vec![1, 2]), (), "send 1:55 [1, 2]".to_string());
        // S→C 0x3E is the desk's own (`units::messages::update_item_stat`
        // through `send`): a GUID without an item unit sends nothing and
        // the rest is not asked.
        {
            let before = d.rest.last();
            d.send_item_stat(m, 58, 59);
            assert_eq!(d.rest.last(), before, "send_item_stat is not forwarded");
        }
        // The item bit stream is the desk's own (`items/bitstream.md`,
        // `wiring::inventory::bits`): the rest is not asked, and a GUID
        // without an item unit has no stream.
        {
            let before = d.rest.last();
            assert_eq!(d.item_bits(60, 61, 62), Vec::<u8>::new());
            assert_eq!(d.rest.last(), before, "item_bits is not forwarded");
            let _ = &a.bits;
        }
        fwd!(
            d.store_messages(m, 63),
            a.msgs,
            "store_messages 1:55 63".to_string()
        );
    });
}

#[test]
fn forward_pending_answers_true() {
    forward_pending(true);
}

#[test]
fn forward_pending_answers_false() {
    forward_pending(false);
}

/// Every `InvWorld` method of `inv_world.rs` that goes to the rest: the
/// unit ids become (type, GUID) / GUID, the answer is unchanged.
fn forward_inv_world(flag: bool) {
    let mut w = World::new();
    let g = w.ground_item(CAP, 11, 11);
    let item = w.unit(g).unwrap();
    let pl = w.player;
    let a = wild(flag, &w);
    w.rest.a = a.clone();
    let p = format!("0:{}", w.pguid());
    w.desk(|d| {
        macro_rules! fwd {
            ($call:expr, $want:expr, $log:expr) => {{
                let got = $call;
                assert_eq!(got, $want, "{}", $log);
                assert_eq!(d.rest.last(), $log);
            }};
        }
        fwd!(
            InvWorld::charm_relink(d, pl, item),
            (),
            format!("charm_relink {p} {g}")
        );
        fwd!(
            InvWorld::stat_refresh(d, pl),
            (),
            format!("stat_refresh {p}")
        );
        fwd!(
            InvWorld::socket_filled(d, item),
            flag,
            format!("socket_filled {g}")
        );
        fwd!(
            InvWorld::inventory_pass(d, pl),
            (),
            format!("inventory_pass {p}")
        );
        fwd!(
            InvWorld::trade_hook(d, pl, item),
            (),
            format!("trade_hook {p} {g}")
        );
        fwd!(
            InvWorld::weapon_in_use_update(d, pl),
            (),
            format!("weapon_in_use_update {p}")
        );
        fwd!(
            InvWorld::stat_link(d, pl, item),
            (),
            format!("stat_link {p} {g}")
        );
        fwd!(
            InvWorld::weapon_bookkeeping(d, pl, item),
            (),
            format!("weapon_bookkeeping {p}")
        );
        fwd!(
            InvWorld::item_active_on(d, item, pl),
            flag,
            format!("item_active_on {g} {p}")
        );
        fwd!(
            InvWorld::own_contribution(d, item, pl, 7),
            909,
            format!("own_contribution {g} {p} 7")
        );
        fwd!(
            InvWorld::one_or_two_handed(d, pl, item),
            flag,
            format!("one_or_two_handed {p} {g}")
        );
        // Answered by the desk (`wiring::inventory::queries`): the
        // item level requirement and the primary type's `shoots`.
        {
            let before = d.rest.last();
            let _ = InvWorld::level_requirement(d, item, pl);
            let _ = InvWorld::ammo_type(d, item);
            let _ = InvWorld::two_handed(d, item);
            assert_eq!(d.rest.last(), before, "not forwarded");
        }
        fwd!(
            InvWorld::has_allowed_location(d, item),
            flag,
            format!("has_allowed_location {g}")
        );
        fwd!(
            InvWorld::quiver_kind(d, item),
            flag,
            format!("quiver_kind {g}")
        );
        // Answered by the desk: the interact info on the unit record
        // (`units::record::InteractInfo`), not forwarded.
        {
            let before = d.rest.last();
            let _ = InvWorld::interaction(d, pl);
            InvWorld::clear_interaction(d, pl);
            assert_eq!(d.rest.last(), before, "not forwarded");
        }
        fwd!(
            InvWorld::player_data_4c(d, pl),
            905,
            format!("player_data_4c {p}")
        );
        fwd!(
            InvWorld::player_data_50(d, pl),
            912,
            format!("player_data_50 {p}")
        );
        fwd!(
            InvWorld::npc_talking(d, item, pl),
            flag,
            format!("npc_talking 4:{g} {p}")
        );
        fwd!(
            InvWorld::player_trade_gate(d, pl),
            Some(flag),
            format!("player_trade_gate {p}")
        );
        // A unit without a record: type NONE, GUID −1.
        fwd!(
            InvWorld::stat_refresh(d, UnitId(9999)),
            (),
            format!("stat_refresh {}:{}", Owner::NONE, u32::MAX)
        );
    });
}

#[test]
fn forward_inv_world_answers_true() {
    forward_inv_world(true);
}

#[test]
fn forward_inv_world_answers_false() {
    forward_inv_world(false);
}

/// The `InvRest` defaults (`wiring/inventory/mod.rs` doc: "Default:
/// fails / none / no / 0 / 1"), which a host that does not override
/// them (the server's `MoveRest` does not) runs on.
#[test]
fn inv_rest_defaults() {
    use d2_sim::wiring::inventory::InvRest;
    struct Bare;
    impl MovePending for Bare {}
    impl InvRest for Bare {
        fn item_active_on(&self, _: u32, _: Owner) -> bool {
            false
        }
        fn own_contribution(&self, _: u32, _: Owner, _: u16) -> i32 {
            0
        }
        fn level_requirement(&self, _: u32, _: Owner) -> i32 {
            -1
        }
        fn two_handed(&self, _: u32) -> bool {
            false
        }
        fn one_or_two_handed(&self, _: Owner, _: u32) -> bool {
            false
        }
        fn ammo_type(&self, _: u32) -> Option<i16> {
            None
        }
        fn has_allowed_location(&self, _: u32) -> bool {
            true
        }
        fn quiver_kind(&self, _: u32) -> bool {
            false
        }
        fn player_data_4c(&self, _: Owner) -> u32 {
            0
        }
        fn player_data_50(&self, _: Owner) -> u32 {
            0
        }
        fn npc_talking(&self, _: Owner, _: Owner) -> bool {
            false
        }
        fn player_trade_gate(&self, _: Owner) -> Option<bool> {
            None
        }
    }
    let mut b = Bare;
    let o = Owner::player(3);
    assert_eq!(InvRest::pos(&b, o), (0, 0));
    InvRest::set_pos(&mut b, o, 5, 6);
    assert_eq!(InvRest::pos(&b, o), (0, 0));
    assert!(b.gold_request(o, 1).is_none());
    assert!(!b.socket_link(1, 2, 3));
    assert!(!b.link_into_item(1, 2));
    assert!(!InvRest::socket_filled(&b, 1));
    assert!(!InvRest::socket_filler(&b, 1));
    assert_eq!(InvRest::spell(&b, 1), 0);
    InvRest::trade_hook(&mut b, o, 1);
}

// ---- inventory outcomes ------------------------------------------------------

/// Placing an item that is still in a room (mode 3) removes it
/// from the room (room delete notice, collision freed, room list), then
/// links it, marks its cells and sets x, y, page and the node fields.
#[test]
fn place_removes_a_ground_item_from_its_room() {
    let mut w = World::new();
    let g = w.ground_item(CAP, 11, 11);
    assert!(w.in_room(g));
    w.rest.take();
    let me = w.me();
    assert!(w.desk(|d| d.place_at(me, g, 0, 3, 1)));
    assert!(!w.in_room(g), "ground item removed from room list");
    let log = w.rest.take();
    let notice = format!("room_delete_notice {g}");
    let coll = format!("free_collision {g}");
    let i = log.iter().position(|l| *l == notice).expect("notice");
    assert_eq!(log.get(i + 1), Some(&coll), "{log:?}");
    let d = w.data(g);
    assert_eq!(
        (d.page, d.x, d.y, d.node_grid, d.node_kind),
        (0, 3, 1, 3, 1)
    );
    assert_eq!(w.inventory().items(), [w.unit(g).unwrap()]);
}

/// An item linked in another inventory is unlinked from it before
/// it is linked into the new one.
#[test]
fn place_unlinks_from_the_old_inventory() {
    let mut w = World::new();
    let m = w.alloc(d2_sim::units::UnitType::Player, 0);
    let mg = w.units.get(m).unwrap().guid;
    w.state.add_inventory(
        m,
        d2_sim::items::inventory::UnitKind::Player { class: 0 },
        mg,
    );
    let g = w.ground_item(CAP, 11, 11);
    let me = w.me();
    assert!(w.desk(|d| d.place_at(Owner::player(mg), g, 0, 0, 0)));
    let u = w.unit(g).unwrap();
    assert_eq!(w.state.inventories[&m].items(), [u]);
    assert_eq!(w.state.inventories[&m].count, 1);
    assert!(w.desk(|d| d.place_at(me, g, 0, 4, 2)));
    assert!(w.state.inventories[&m].items().is_empty(), "unlinked");
    assert_eq!(w.state.inventories[&m].count, 0);
    assert_eq!(w.data(g).inv, Some(w.player));
    assert_eq!(w.inventory().items(), [u]);
}

/// Unit flag 0x2 (targetable) is cleared; no other
/// unit flag changes.
#[test]
fn insert_clears_only_the_targetable_flag() {
    let mut w = World::new();
    let g = w.cursor_item(CAP);
    let u = w.unit(g).unwrap();
    let before = w.units.get(u).unwrap().flags | 0x2 | 0x100;
    w.units.get_mut(u).unwrap().flags = before;
    assert_eq!(w.handle(&insert(g, 0, 0, 0)), Ok(0));
    assert_eq!(w.units.get(u).unwrap().flags, before & !0x2);
}

/// The link check sockets the item when the inventory
/// belongs to an item (`socket_link` supplies the result);
/// any other owner succeeds without it.
#[test]
fn link_check_sockets_only_into_an_item() {
    let mut w = World::new();
    let target = w.ground_item(SWORD, 11, 11);
    let filler = w.ground_item(KEY, 12, 12);
    let me = w.me();
    for ok in [false, true] {
        w.rest.a.flag = ok;
        w.rest.take();
        assert_eq!(
            w.desk(|d| InventoryOps::link_check(d, Owner::item(target), filler, 1)),
            ok
        );
        assert_eq!(w.rest.take(), [format!("socket_link {target} {filler} 1")]);
    }
    w.rest.a.flag = false;
    assert!(w.desk(|d| InventoryOps::link_check(d, me, filler, 1)));
    assert_eq!(w.rest.called("socket_link"), 0);
}

/// The player's page 4 is record 8 (6 × 4) in a classic game and
/// record 12 (6 × 8) in an expansion game, selected by `GameFields::expansion`.
#[test]
fn stash_size_follows_the_game_type() {
    for (expansion, fits) in [(false, false), (true, true)] {
        let mut w = World::new();
        w.fields.expansion = expansion;
        let g = w.ground_item(KEY, 11, 11);
        let me = w.me();
        assert_eq!(w.desk(|d| d.place_at(me, g, 4, 5, 7)), fits, "y 7");
        assert!(w.desk(|d| d.place_at(me, g, 4, 5, 3)), "y 3 fits both");
    }
}

/// Stack compatibility requires matching quality, file index, ethereal
/// flag and damage stats (21–24, 159, 160).
#[test]
fn stack_test_compares_quality_file_index_ethereal_and_damage() {
    let mut w = World::new();
    let a = w.ground_item(KNIFE, 11, 11);
    let b = w.ground_item(KNIFE, 12, 12);
    assert!(w.desk(|d| InventoryOps::stack_test(d, a, b)));
    let ub = w.unit(b).unwrap();
    for s in [21u16, 22, 23, 24, 159, 160] {
        let ua = w.unit(a).unwrap();
        w.set_stat(ua, s, 5);
        w.set_stat(ub, s, 4);
        assert!(!w.desk(|d| InventoryOps::stack_test(d, a, b)), "stat {s}");
        w.set_stat(ub, s, 5);
        assert!(w.desk(|d| InventoryOps::stack_test(d, a, b)), "stat {s}");
    }
    let ua = w.unit(a).unwrap();
    type Edit = fn(&mut d2_sim::items::Item<()>, bool);
    let edits: [(&str, Edit); 3] = [
        ("quality", |i, b| i.quality = if b { 3 } else { 2 }),
        ("file index", |i, b| i.file_index = if b { 6 } else { 5 }),
        ("ethereal", |i, b| {
            i.flags = if b {
                i.flags | 0x40_0000
            } else {
                i.flags & !0x40_0000
            }
        }),
    ];
    for (what, f) in edits {
        f(w.items.get_mut(ua).unwrap(), false);
        f(w.items.get_mut(ub).unwrap(), true);
        assert!(!w.desk(|d| InventoryOps::stack_test(d, a, b)), "{what}");
        f(w.items.get_mut(ub).unwrap(), false);
        assert!(w.desk(|d| InventoryOps::stack_test(d, a, b)), "{what}");
    }
}

/// p = item stat 91; p ≠ 0 → bonus = reqstr × p / 100.
/// Strength 10 against reqstr 20: fails at p = 0, passes at p = −50.
#[test]
fn requirement_percent_lowers_the_strength_needed() {
    let mut w = World::new();
    let g = w.ground_item(HEAVY_CAP, 11, 11);
    let me = w.me();
    assert!(!w.desk(|d| d.requirements(g, me, false)));
    let u = w.unit(g).unwrap();
    w.set_stat(u, 91, -50);
    assert!(w.desk(|d| d.requirements(g, me, false)));
    w.set_stat(u, 91, -49);
    assert!(
        !w.desk(|d| d.requirements(g, me, false)),
        "20 − 9 = 11 > 10"
    );
}

/// Equipping an incompatible two-handed item returns 7 when the other
/// hand's item fits in page 0, else 0. The test fills that page to exercise
/// both outcomes.
#[test]
fn hand_result_7_needs_room_for_the_other_hand() {
    for full in [false, true] {
        let mut w = World::new();
        let t = w.cursor_item(SWORD);
        assert_eq!(w.handle(&body(0x1A, t, 4)), Ok(0));
        w.drain();
        let x = w.cursor_item(SHIELD);
        assert_eq!(w.handle(&body(0x1A, x, 5)), Ok(0));
        w.drain();
        if full {
            for i in 0..10u32 {
                let c = w.cursor_item(CAP);
                assert_eq!(w.handle(&insert(c, (i % 5) * 2, (i / 5) * 2, 0)), Ok(0));
                w.drain();
            }
        }
        let n = w.cursor_item(TWO_HANDER);
        w.inv.items[TWO_HANDER].twohanded = 1;
        let me = w.me();
        assert_eq!(
            w.desk(|d| d.equip_check(me, 4, Some(n), true)),
            if full { 0 } else { 7 }
        );
    }
}

/// Ground-or-owned check, mode 3: within 10 subtiles per axis of the
/// player → 0, else 1; the item's position is its own.
#[test]
fn ground_check_measures_from_the_item_position() {
    let mut w = World::new();
    let me = w.me();
    w.rest.pos.insert(me, (100, 100));
    let near = w.ground_item(CAP, 110, 90);
    let far = w.ground_item(CAP, 111, 100);
    let far_y = w.ground_item(CAP, 100, 89);
    assert_eq!(
        w.desk(|d| InventoryOps::check_ground_or_owned(d, me, near)),
        0
    );
    assert_eq!(
        w.desk(|d| InventoryOps::check_ground_or_owned(d, me, far)),
        1
    );
    assert_eq!(
        w.desk(|d| InventoryOps::check_ground_or_owned(d, me, far_y)),
        1
    );
}

/// A cap stored at (0, 0) of page 0 through 0x18, messages drained.
fn stored_cap(w: &mut World) -> u32 {
    let g = w.cursor_item(CAP);
    assert_eq!(w.handle(&insert(g, 0, 0, 0)), Ok(0));
    w.drain();
    g
}

/// Every item of the owner's list with item flag 0x4 loses it;
/// `targeting_probe` is asked about the owner, never the item: a player owner
/// queues S→C 0x3F (code 0xFF, the item's GUID, 0xFFFF) for each
/// item, a mercenary (monster) owner none.
#[test]
fn targeting_reset_clears_and_queues_0x3f() {
    let mut w = World::new();
    let g = stored_cap(&mut w);
    let u = w.unit(g).unwrap();
    w.items.get_mut(u).unwrap().flags |= 0x4 | 0x100;
    w.rest.sent.clear();
    let me = w.me();
    w.desk(|d| d.targeting_reset(me));
    assert_eq!(w.items.get(u).unwrap().flags & 0x104, 0x100);
    let mut want = vec![0x3F, 0xFF];
    want.extend_from_slice(&g.to_le_bytes());
    want.extend_from_slice(&[0xFF, 0xFF]);
    let sent: Vec<_> = w.rest.sent.clone();
    assert_eq!(sent, [(me, want)]);

    // `targeting_probe` returns the unit's type: player 0, monster 1, item 4,
    // missing unit 6.
    // A desert mercenary (monstats row 338, enabled).
    w.data.monsters = vec![MonsterInfo::default(); 339];
    w.data.monsters[338].enabled = true;
    let merc = w.alloc(UnitType::Monster, 338);
    let p = w.player;
    w.desk(|d| {
        assert_eq!(InvWorld::targeting_probe(d, p), 0);
        assert_eq!(InvWorld::targeting_probe(d, merc), 1);
        assert_eq!(InvWorld::targeting_probe(d, u), 4);
        assert_eq!(InvWorld::targeting_probe(d, UnitId(9999)), 6);
    });

    // The same flagged item in a mercenary's inventory: the flag is
    // cleared, nothing is queued (the item is not the probed unit).
    let mguid = w.units.get(merc).unwrap().guid;
    w.state
        .add_inventory(merc, UnitKind::Monster { class: 338 }, mguid);
    let c = w.cursor_item(CAP);
    let cu = w.unit(c).unwrap();
    assert!(w.desk(|d| d.remove(p, cu)));
    assert!(w.desk(|d| d.place(merc, cu, (0, 0), true, false)));
    assert!(w.state.holds(merc, cu));
    w.items.get_mut(cu).unwrap().flags |= 0x4;
    w.rest.sent.clear();
    w.desk(|d| d.reset_targeting(merc));
    assert_eq!(w.items.get(cu).unwrap().flags & 0x4, 0);
    assert!(w.rest.sent.is_empty());
}

/// Ground-or-owned check, mode 3: an item in another act than the player
/// → 2.
#[test]
fn ground_check_other_act_is_2() {
    let mut w = World::new();
    let me = w.me();
    let g = w.ground_item(CAP, 11, 11);
    assert_eq!(w.desk(|d| InventoryOps::check_ground_or_owned(d, me, g)), 0);
    let u = w.unit(g).unwrap();
    w.units.get_mut(u).unwrap().act = 1;
    assert_eq!(w.desk(|d| InventoryOps::check_ground_or_owned(d, me, g)), 2);
    let p = w.player;
    w.units.get_mut(p).unwrap().act = 1;
    assert_eq!(w.desk(|d| InventoryOps::check_ground_or_owned(d, me, g)), 0);
}

/// Stack compatibility requires neither item to have sockets (stat 194).
#[test]
fn stack_test_refuses_socketed_items() {
    let mut w = World::new();
    let a = w.ground_item(KNIFE, 11, 11);
    let b = w.ground_item(KNIFE, 12, 12);
    for (x, y) in [(a, b), (b, a)] {
        let u = w.unit(x).unwrap();
        w.set_stat(u, 194, 1);
        assert!(!w.desk(|d| InventoryOps::stack_test(d, x, y)));
        assert!(!w.desk(|d| InventoryOps::stack_test(d, y, x)));
        w.set_stat(u, 194, 0);
        assert!(w.desk(|d| InventoryOps::stack_test(d, x, y)));
    }
}

/// The inventory getters of `InventoryOps`: an owner
/// without an inventory has none; the item list in link order as GUIDs;
/// the optional GUID of the weapon in use.
#[test]
fn inventory_getters() {
    let mut w = World::new();
    let a = stored_cap(&mut w);
    let b = w.cursor_item(KEY);
    assert_eq!(w.handle(&insert(b, 5, 0, 0)), Ok(0));
    w.drain();
    let me = w.me();
    let ga = w.ground_item(CAP, 11, 11);
    w.desk(|d| {
        assert!(d.has_inventory(me));
        assert!(!d.has_inventory(Owner::item(ga)));
        assert_eq!(d.items(me), [a, b]);
        assert!(d.items(Owner::item(ga)).is_empty());
        assert_eq!(d.weapon_in_use(me), None);
    });
    let p = w.player;
    w.state.inventories.get_mut(&p).unwrap().weapon_guid = b;
    assert_eq!(w.desk(|d| d.weapon_in_use(me)), Some(b));
}

/// `beltable` reads the item type's `beltable` column.
#[test]
fn beltable_reads_the_item_type() {
    let mut w = World::new();
    let hp = w.ground_item(HP1, 11, 11);
    let key = w.ground_item(KEY, 12, 12);
    w.desk(|d| {
        assert!(d.beltable(hp));
        assert!(!d.beltable(key));
        assert!(!d.beltable(0xDEAD), "no such item");
    });
}

/// The `InventoryOps` calls that go to the rest: arguments and answer
/// unchanged.
#[test]
fn forward_inventory_ops() {
    for flag in [false, true] {
        let mut w = World::new();
        let g = w.ground_item(TWO_HANDER, 11, 11);
        w.rest.a.flag = flag;
        // Two-handed is the items column (`queries`), not the rest's.
        w.inv.items[TWO_HANDER].twohanded = u8::from(flag);
        w.desk(|d| {
            assert_eq!(d.link_into_item(7, 8), flag);
            assert_eq!(d.rest.last(), "link_into_item 7 8");
            assert_eq!(InventoryOps::two_handed(d, g), flag);
            assert_eq!(d.rest.last(), "link_into_item 7 8", "not forwarded");
        });
    }
}

/// `clear_body_slot` logs a slot that still holds an item
/// (`InvError::BodySlotHeld`); an empty slot is not logged.
#[test]
fn clear_body_slot_logs_a_held_slot() {
    use d2_sim::wiring::inventory::InvError;
    let mut w = World::new();
    let s = w.cursor_item(SWORD);
    assert_eq!(w.handle(&body(0x1A, s, 4)), Ok(0));
    let me = w.me();
    w.desk(|d| d.clear_body_slot(me, 5));
    assert!(w.state.errors.is_empty());
    w.desk(|d| d.clear_body_slot(me, 4));
    assert_eq!(
        w.state.errors,
        [InvError::BodySlotHeld(w.unit(s).unwrap(), 4)]
    );
}

/// Auto-equip returns the empty body location when both allowed locations
/// are identical (helm: 1).
#[test]
fn auto_equip_takes_the_single_location() {
    let mut w = World::new();
    let g = w.ground_item(CAP, 11, 11);
    let me = w.me();
    assert_eq!(w.desk(|d| d.auto_equip(me, g, true)), Some(1));
}

/// "Stored or equipped" and "belt item": 1 only for an item in mode
/// 0/1 (resp. 2) that is not in the player's inventory; else 0.
#[test]
fn stored_or_equipped_and_belt_checks() {
    let mut w = World::new();
    let other = w.alloc(d2_sim::units::UnitType::Player, 0);
    let og = w.units.get(other).unwrap().guid;
    w.state.add_inventory(
        other,
        d2_sim::items::inventory::UnitKind::Player { class: 0 },
        og,
    );
    let me = w.me();
    let mine = stored_cap(&mut w);
    let theirs = w.ground_item(CAP, 11, 11);
    assert!(w.desk(|d| d.place_at(Owner::player(og), theirs, 0, 0, 0)));
    w.desk(|d| d.set_mode(theirs, 0));
    let hp = w.ground_item(HP1, 12, 12);
    assert!(w.desk(|d| d.belt_place(Owner::player(og), hp, 0)));
    w.desk(|d| d.set_mode(hp, 2));
    w.desk(|d| {
        assert_eq!(d.check_stored_or_equipped(me, mine), 0);
        assert_eq!(d.check_stored_or_equipped(me, theirs), 1);
        assert_eq!(d.check_belt(me, hp), 1);
        assert_eq!(d.check_belt(me, mine), 0, "mode 0");
    });
}

/// Trading check: the interaction is with a player unit (type 0).
#[test]
fn trading_is_an_interaction_with_a_player() {
    let mut w = World::new();
    let me = w.me();
    assert!(!w.desk(|d| d.trading(me)));
    let (p, pg) = (w.player, w.pguid());
    // The interact info on the player's record: (type, GUID), active.
    let stage = |w: &mut World, ty: u8| {
        let r = w.units.get_mut(p).unwrap();
        r.interact.reset();
        r.interact.set(ty, pg);
    };
    stage(&mut w, 0);
    assert!(w.desk(|d| d.trading(me)));
    assert_eq!(
        w.desk(|d| InvWorld::interaction(d, p)),
        InteractionTarget::Unit { ty: 0, unit: p }
    );
    stage(&mut w, 1);
    assert!(!w.desk(|d| d.trading(me)), "no monster with that GUID");
    assert_eq!(
        w.desk(|d| InvWorld::interaction(d, p)),
        InteractionTarget::Missing
    );
    w.desk(|d| InvWorld::clear_interaction(d, p));
    assert_eq!(w.units.get(p).unwrap().interact.get(), None);
}

/// With no interaction, a nonzero `player_data_4c` seam result refuses a move.
#[test]
fn item_move_gate_refuses_with_player_data_4c() {
    let mut w = World::new();
    let me = w.me();
    assert!(w.desk(|d| d.item_move_gate(me, None)));
    w.rest.a.code = 1;
    assert!(!w.desk(|d| d.item_move_gate(me, None)));
}

/// Ground placement's `add_to_room` inserts an item with no room into
/// the spot's room. An item already in that room is left; one in another
/// room is logged
/// (`InvError::OtherRoom`) and left.
#[test]
fn add_to_room_by_room() {
    use d2_sim::wiring::inventory::InvError;
    let mut w = World::new();
    let r2 = w.game.lists.create_room(0).unwrap();
    let g = w.ground_item(CAP, 11, 11);
    let u = w.unit(g).unwrap();
    let room = w.room;
    let spot = |room| Spot { room, x: 1, y: 1 };
    w.desk(|d| d.add_to_room(g, spot(room)));
    assert!(w.state.errors.is_empty());
    assert_eq!(w.game.lists.unit(u).unwrap().room(), Some(room));
    w.desk(|d| d.add_to_room(g, spot(r2)));
    assert_eq!(w.state.errors, [InvError::OtherRoom(u)]);
    assert_eq!(w.game.lists.unit(u).unwrap().room(), Some(room));
    w.state.errors.clear();
    w.desk(|d| {
        MovePending::remove_from_room(d, g);
        assert!(!d.in_room(g));
        d.add_to_room(g, spot(r2));
        assert!(d.in_room(g));
    });
    assert_eq!(w.game.lists.unit(u).unwrap().room(), Some(r2));
    assert!(w.state.errors.is_empty());
    // A list error (an unknown room) is logged, not dropped.
    let bad = d2_sim::units::RoomId(999);
    w.desk(|d| {
        MovePending::remove_from_room(d, g);
        d.add_to_room(g, spot(bad));
    });
    assert_eq!(
        w.state.errors,
        [InvError::List(
            d2_sim::units::lists::ListError::UnknownRoom(bad)
        )]
    );
}

/// `queue_update` adds the unit to its room's update queue.
#[test]
fn queue_update_queues_the_unit_in_its_room() {
    let mut w = World::new();
    let g = w.ground_item(CAP, 11, 11);
    let u = w.unit(g).unwrap();
    let room = w.room;
    w.game.lists.unqueue_update(u).unwrap();
    assert!(!w.game.lists.update_queue(room).contains(&u));
    w.desk(|d| d.queue_update(Owner::item(g)));
    assert!(w.game.lists.update_queue(room).contains(&u));
}

/// Merging requires a durability-bearing item and stat 152
/// (`item_indesctructible`) below 1.
#[test]
fn merge_allowed_needs_stat_152_below_1() {
    let mut w = World::new();
    let k = w.ground_item(KNIFE, 11, 11);
    assert!(w.desk(|d| d.merge_allowed(k)));
    let u = w.unit(k).unwrap();
    w.set_stat(u, 152, 1);
    assert!(!w.desk(|d| d.merge_allowed(k)));
    let n = w.ground_item(NODUR_KEY, 12, 12);
    assert!(!w.desk(|d| d.merge_allowed(n)), "nodurability");
    let g = w.ground_item(GOLD, 13, 13);
    assert!(!w.desk(|d| d.merge_allowed(g)), "durability 0");
}

/// `alive` rejects a missing unit or a dead player; modes 0 and 17
/// are checked here against neutral mode 1.
#[test]
fn alive_is_not_dead() {
    let mut w = World::new();
    let me = w.me();
    let p = w.player;
    // The allocator leaves a player in mode 0 (death); mode 1 is neutral.
    assert!(!w.desk(|d| d.alive(me)));
    w.units.get_mut(p).unwrap().mode = 1;
    assert!(w.desk(|d| d.alive(me)));
    w.units.get_mut(p).unwrap().mode = 17;
    assert!(!w.desk(|d| d.alive(me)));
    assert!(!w.desk(|d| d.alive(Owner::player(0xDEAD))), "no unit");
}

/// `MoveUnits` reads and writes body location, stored page, owning inventory
/// and unit update bits. Item metadata and the game expansion flag are
/// checked through the same adapter.
#[test]
fn move_units_fields() {
    let mut w = World::new();
    let me = w.me();
    let cap = stored_cap(&mut w);
    let key = w.ground_item(KEY, 11, 11);
    let hp = w.ground_item(HP1, 12, 12);
    let p = w.player;
    w.units.get_mut(p).unwrap().flags2 = 0x6;
    let ucap = w.unit(cap).unwrap();
    w.items.get_mut(ucap).unwrap().file_index = 7;
    w.desk(|d| {
        assert!(d.unit_exists(me));
        assert!(!d.unit_exists(Owner::player(0xDEAD)));
        assert_eq!(d.unit_class(me), CLASS);
        assert_eq!(d.update_bits(me), 6);
        assert!(MoveUnits::expansion(d));
        d.set_stored_page(cap, 3);
        assert_eq!(d.stored_page(cap), 3);
        d.set_body_loc(cap, 9);
        assert_eq!(d.body_loc(cap), 9);
        assert_eq!(d.item_owner(cap), Some(me));
        assert_eq!(d.item_owner(key), None);
        assert_eq!(d.code(cap), *b"cap ");
        assert_eq!(MoveUnits::quality(d, cap), d2_sim::items::q::NORMAL);
        assert_eq!(d.file_index(cap), 7);
        assert_eq!(d.file_index(0xDEAD), -1);
        assert_eq!(d.quest(key), 3);
        assert_eq!(d.component(key), 5);
        assert!(d.useable(hp));
        assert!(!d.useable(cap));
    });
    let d = w.data(cap);
    assert_eq!((d.stored_page, d.body_loc), (3, 9));
    w.fields.expansion = false;
    assert!(!w.desk(|d| MoveUnits::expansion(d)));
}

/// Socket getters read stat 194 and the item's inventory in link order.
/// Filled / filler tests forward to the rest; the spell reads suffix slot 0.
#[test]
fn socket_getters() {
    let mut w = World::new();
    let s = w.ground_item(SWORD, 11, 11);
    let a = w.ground_item(KEY, 12, 12);
    let b = w.ground_item(KEY, 13, 13);
    let su = w.unit(s).unwrap();
    w.set_stat(su, 194, 3);
    w.state
        .add_inventory(su, d2_sim::items::inventory::UnitKind::Item, s);
    let (ua, ub) = (w.unit(a).unwrap(), w.unit(b).unwrap());
    w.desk(|d| {
        let mut inv = d.state.inventories.remove(&su).unwrap();
        inv.link(d, ub, None);
        inv.link(d, ua, None);
        d.state.inventories.insert(su, inv);
    });
    for flag in [false, true] {
        w.rest.a.flag = flag;
        w.rest.a.number = if flag { 33 } else { -7 };
        w.desk(|d| {
            assert_eq!(d.sockets(s), 3);
            assert_eq!(d.fillers(s), [b, a]);
            assert!(d.fillers(a).is_empty());
            assert_eq!(MoveUnits::socket_filled(d, s), flag);
            assert_eq!(d.rest.last(), format!("socket_filled {s}"));
            assert_eq!(MoveUnits::socket_filler(d, a), flag);
            assert_eq!(d.rest.last(), format!("socket_filler {a}"));
            let before = d.rest.last();
            assert_eq!(MoveUnits::spell(d, a), 0);
            assert_eq!(d.rest.last(), before, "spell is not forwarded");
        });
    }
}
