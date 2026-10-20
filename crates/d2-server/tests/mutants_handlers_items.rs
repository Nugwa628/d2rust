//! Mutation-testing gaps of the cube handlers on the merged host:
//! `items::cube_world::ServerCube` (the cube's world over the economy wiring).
//! Each test transmutes (C→S 0x4F button 0x18 through `SimGame::handle`) one ring
//! in the open cube and asserts the cube's rules: whether the recipe is eligible
//! (version / expansion, ladder / game type, min difficulty, class; date and stat
//! ops), the type pick's draws on the game seed and its item-format filter, the
//! quantity written after creation, and the ear's player name and hardcore flag.
//!
//! Integration test of `d2-server`; it shares the `mutants_handlers_fx` cube host
//! and guards the cube wiring that the server's economy host provides.

mod mutants_handlers_fx;

use d2_server::adapters::handlers::items::CreationInfo;
use d2_server::seams::ResultCode;
use d2_sim::items::{flag, q};
use d2_sim::rng::Seed;
use d2_sim::wiring::economy::GameFields;
use d2_sim::world::cube::{input_flags, kind, output_flags, InputSlot, OutputSlot, Recipe};
use mutants_handlers_fx::{ring_to_amulet, CubeFx, AMULET, EAR, GAME_SEED, HST, RING};

fn fields(expansion: bool) -> GameFields {
    GameFields::new(Seed::init_low(GAME_SEED), expansion)
}

/// Transmutes with `recipe` on a fresh host (player class `class`,
/// `fields`); whether the ring was used.
fn matches(class: u32, fields: GameFields, recipe: Recipe) -> bool {
    CubeFx::new(class, fields, vec![recipe]).transmute()
}

/// A record with `version` ≥ 100 needs an expansion game.
#[test]
fn version_needs_expansion() {
    let r = Recipe {
        version: 100,
        ..ring_to_amulet()
    };
    assert!(!matches(2, fields(false), r.clone()));
    assert!(matches(2, fields(true), r));
    let classic = Recipe {
        version: 99,
        ..ring_to_amulet()
    };
    assert!(matches(2, fields(false), classic));
}

/// A `ladder` record needs game type ≠ 0 or a ladder game.
#[test]
fn ladder_record_needs_ladder_or_game_type() {
    let r = Recipe {
        ladder: 1,
        ..ring_to_amulet()
    };
    assert!(!matches(2, fields(true), r.clone()));
    let typed = GameFields {
        game_type: 3,
        ..fields(true)
    };
    assert!(matches(2, typed, r.clone()));
    let ladder = GameFields {
        ladder: true,
        ..fields(true)
    };
    assert!(matches(2, ladder, r));
}

/// `min diff` ≤ game difficulty.
#[test]
fn min_difficulty() {
    let r = |min_diff| Recipe {
        min_diff,
        ..ring_to_amulet()
    };
    let diff = |difficulty| GameFields {
        difficulty,
        ..fields(true)
    };
    assert!(!matches(2, diff(0), r(1)));
    assert!(matches(2, diff(1), r(1)));
    assert!(!matches(2, diff(1), r(2)));
    assert!(matches(2, diff(2), r(2)));
}

/// `class` 0xFF or the player's class id.
#[test]
fn class_record() {
    let r = Recipe {
        class: 2,
        ..ring_to_amulet()
    };
    assert!(matches(2, fields(true), r.clone()));
    assert!(!matches(3, fields(true), r));
}

/// Date op 1: `param` ≤ day of month ≤ `value`; op 2: day of week + 1 =
/// `value`. The staged date is the 15th, day of week + 1 = 3.
#[test]
fn date_ops() {
    let op = |op, param, value| Recipe {
        op,
        param,
        value,
        ..ring_to_amulet()
    };
    assert!(matches(2, fields(true), op(1, 15, 15)));
    assert!(!matches(2, fields(true), op(1, 16, 20)));
    assert!(!matches(2, fields(true), op(1, 2, 14)));
    assert!(matches(2, fields(true), op(2, 0, 3)));
    assert!(!matches(2, fields(true), op(2, 0, 1)));
}

/// Stat op 3: the player's stat (value, layer 0) ≥ `value` (shift 0).
#[test]
fn stat_op() {
    let r = Recipe {
        op: 3,
        param: 12,
        value: 5,
        ..ring_to_amulet()
    };
    for (level, used) in [(5, true), (4, false)] {
        let mut fx = CubeFx::new(2, fields(true), vec![r.clone()]);
        let p = fx.player;
        fx.set_stat(p, 12, level);
        assert_eq!(fx.transmute(), used, "level {level}");
    }
}

/// The type pick draws `roll(N)` then `roll(count)` on the game
/// seed before the item request; only items whose `version` < 100 or
/// with a game item format ≥ 100 are candidates. Run against the same
/// output by item code on a seed advanced by those two draws: the same
/// amulet, the same game seed afterwards.
#[test]
fn type_pick_draws_on_the_game_seed() {
    let by_type = Recipe {
        outputs: [
            OutputSlot {
                kind: kind::ITEMTYPE,
                item: mutants_handlers_fx::T_AMULET,
                lvl: 10,
                ..ring_to_amulet().outputs[0]
            },
            Default::default(),
            Default::default(),
        ],
        ..ring_to_amulet()
    };
    let mut a = CubeFx::new(2, fields(true), vec![by_type]);
    a.parts().cube.items[AMULET].version = 100;
    // A game seed whose first draw starts the scan at 0, 1 or 2: the
    // amulet (record 2) is examined (the record before `start` never is).
    let n = a.parts().cube.items.len() as i32;
    let before = (1..)
        .map(Seed::init_low)
        .find(|s| {
            let mut t = *s;
            t.roll(n) <= 2
        })
        .unwrap();
    a.sim.events.sys.hooks.game_seed = before;
    assert!(a.transmute());
    let amu = a.output();
    assert_eq!(
        a.sim.events.sys.hooks.items.get(amu).unwrap().record,
        AMULET
    );

    // The draws: N records; one candidate.
    let mut s = before;
    s.roll(n);
    s.roll(1);
    let by_code = Recipe {
        outputs: [
            OutputSlot {
                lvl: 10,
                ..ring_to_amulet().outputs[0]
            },
            Default::default(),
            Default::default(),
        ],
        ..ring_to_amulet()
    };
    let mut b = CubeFx::new(2, fields(true), vec![by_code]);
    b.sim.events.sys.hooks.game_seed = s;
    assert!(b.transmute());
    assert_eq!(
        a.sim.events.sys.hooks.game_seed,
        b.sim.events.sys.hooks.game_seed
    );
}

/// Last step: a stackable output without `sock` gets stat 70 (quantity) =
/// min(quantity, max stack).
#[test]
fn output_quantity() {
    let r = Recipe {
        outputs: [
            OutputSlot {
                quantity: 5,
                ..ring_to_amulet().outputs[0]
            },
            Default::default(),
            Default::default(),
        ],
        ..ring_to_amulet()
    };
    let mut fx = CubeFx::new(2, fields(true), vec![r]);
    let rec = &mut fx.parts().cube.items[AMULET];
    (rec.stackable, rec.maxstack) = (1, 20);
    assert!(fx.transmute());
    let amu = fx.output();
    let g = &mut fx.sim;
    let qty = g.events.with(&mut g.game, |_, v| v.stat(amu, 70));
    assert_eq!(qty, 5);
}

/// Through the cube's item request an ear carries the player's name and its
/// NAMED flag is the client's hardcore flag.
#[test]
fn ear_gets_the_players_name() {
    for hardcore in [true, false] {
        let r = Recipe {
            outputs: [
                OutputSlot {
                    item: EAR as u16,
                    ..ring_to_amulet().outputs[0]
                },
                Default::default(),
                Default::default(),
            ],
            ..ring_to_amulet()
        };
        let mut fx = CubeFx::new(2, fields(true), vec![r]);
        let p = fx.player;
        let mut name = [0u8; 16];
        name[..6].copy_from_slice(b"Rakkis");
        fx.parts().creation.insert(
            p,
            CreationInfo {
                name,
                hardcore: Some(hardcore),
            },
        );
        assert!(fx.transmute());
        let ear = fx.output();
        let it = fx.sim.events.sys.hooks.items.get(ear).unwrap();
        assert_eq!((it.record, it.quality), (EAR, q::NORMAL));
        assert_eq!(it.name, name);
        assert_eq!(it.flags & flag::NAMED != 0, hardcore);
    }
}

// ---- input tests --------------------------------------------------------------------------

/// A ring-to-amulet record whose slot 0 is changed by `f`.
fn slot(f: impl FnOnce(&mut InputSlot)) -> Recipe {
    let mut r = ring_to_amulet();
    f(&mut r.inputs[0]);
    r
}

/// A fresh host with `recipe`; `setup` runs on it before the transmute.
fn run(recipe: Recipe, setup: impl FnOnce(&mut CubeFx)) -> bool {
    let mut fx = CubeFx::new(2, fields(true), vec![recipe]);
    setup(&mut fx);
    fx.transmute()
}

/// Input tests: item type (flag 0x0002), quality, unique / set file
/// index (`special` − 1), sockets (stat 194: `nos` = 0, `sock` ≠ 0),
/// ethereal (`eth` / `noe`).
#[test]
fn input_item_tests() {
    use input_flags as f;
    let by_type = |t| {
        slot(|s| {
            (s.flags, s.item) = (f::ITEMCODE, t);
        })
    };
    assert!(run(by_type(mutants_handlers_fx::T_RING), |_| {}));
    assert!(!run(by_type(mutants_handlers_fx::T_AMULET), |_| {}));

    let quality = |qv| slot(|s| s.quality = qv);
    assert!(run(quality(q::NORMAL), |_| {}));
    assert!(!run(quality(q::MAGIC), |_| {}));

    let special = |n| {
        slot(|s| {
            (s.flags, s.special) = (f::USEANY | f::SPECIAL, n);
        })
    };
    let file_index_6 = |fx: &mut CubeFx| {
        let r = fx.ring;
        fx.sim.events.sys.hooks.items.get_mut(r).unwrap().file_index = 6;
    };
    assert!(run(special(7), file_index_6));
    assert!(!run(special(3), file_index_6));

    let sockets = |flag| slot(|s| s.flags = f::USEANY | flag);
    let two_sockets = |fx: &mut CubeFx| {
        let r = fx.ring;
        fx.set_stat(r, 194, 2);
    };
    assert!(run(sockets(f::NOS), |_| {}));
    assert!(!run(sockets(f::NOS), two_sockets));
    assert!(run(sockets(f::SOCK), two_sockets));
    assert!(!run(sockets(f::SOCK), |_| {}));

    let ethereal = |fx: &mut CubeFx| {
        let r = fx.ring;
        fx.sim.events.sys.hooks.items.get_mut(r).unwrap().flags |= flag::ETHEREAL;
    };
    assert!(run(sockets(f::ETH), ethereal));
    assert!(!run(sockets(f::ETH), |_| {}));
    assert!(!run(sockets(f::NOE), ethereal));
    assert!(run(sockets(f::NOE), |_| {}));
}

/// Stat op 27 (input 0): the slot-0 item's file index ≠ `value`.
#[test]
fn file_index_op() {
    let op = |value| Recipe {
        op: 27,
        value,
        ..ring_to_amulet()
    };
    let file_index_6 = |fx: &mut CubeFx| {
        let r = fx.ring;
        fx.sim.events.sys.hooks.items.get_mut(r).unwrap().file_index = 6;
    };
    assert!(!run(op(6), file_index_6));
    assert!(run(op(5), file_index_6));
}

/// The captured level is the slot-0 item's level
/// (raised to 1 on the item when < 1); `ilvl` 100 → the output level is
/// that level.
#[test]
fn captured_level() {
    let r = Recipe {
        outputs: [
            OutputSlot {
                ilvl: 100,
                ..ring_to_amulet().outputs[0]
            },
            Default::default(),
            Default::default(),
        ],
        ..ring_to_amulet()
    };
    let mut fx = CubeFx::new(2, fields(true), vec![r.clone()]);
    let ring = fx.ring;
    fx.sim.events.sys.hooks.items.get_mut(ring).unwrap().ilvl = 7;
    assert!(fx.transmute());
    let amu = fx.output();
    assert_eq!(fx.sim.events.sys.hooks.items.get(amu).unwrap().ilvl, 7);

    // Level 0, and an output that cannot be created: the ring stays,
    // raised to level 1.
    let mut bad = r;
    bad.outputs[0].item = 99;
    let mut fx = CubeFx::new(2, fields(true), vec![bad]);
    fx.sim.events.sys.hooks.items.get_mut(ring).unwrap().ilvl = 0;
    assert!(!fx.transmute());
    assert_eq!(fx.sim.events.sys.hooks.items.get(ring).unwrap().ilvl, 1);
}

// ---- outputs ------------------------------------------------------------------------------

fn output(o: OutputSlot) -> Recipe {
    Recipe {
        outputs: [o, Default::default(), Default::default()],
        ..ring_to_amulet()
    }
}

fn amulet_out() -> OutputSlot {
    ring_to_amulet().outputs[0]
}

/// A host with one recipe; its item copies are the inventory model's
/// (`InvDesk::copy_of`, the same copy the vendors use).
fn with_copy(recipe: Recipe) -> CubeFx {
    CubeFx::new(2, fields(true), vec![recipe])
}

/// No copy went to the rest (`ItemPending::duplicate`).
fn no_rest_copy(log: &[String]) {
    assert!(!log.iter().any(|l| l.starts_with("duplicate")), "{log:?}");
}

/// `mod` with an item code: the copy of the slot-0 item (the ring,
/// a new unit) gets the output class and is initialised; it is the
/// output (mode 4 → placed).
#[test]
fn mod_copy_takes_the_output_class() {
    let r = output(OutputSlot {
        flags: output_flags::MOD,
        item: AMULET as u16,
        ..amulet_out()
    });
    let mut fx = with_copy(r);
    let ring = fx.ring;
    assert!(fx.transmute());
    let copy = fx.output();
    assert_ne!(copy, ring);
    assert_eq!(
        fx.sim.events.sys.hooks.items.get(copy).unwrap().record,
        AMULET
    );
    no_rest_copy(&fx.log.take());
}

/// `useitem` with quality 9: the copy of the ring is the output (mode 4, then
/// stored by the placement step: mode 0, linked); both tempered rolls non-zero →
/// quality 9 with that prefix and suffix.
#[test]
fn useitem_tempered() {
    let r = output(OutputSlot {
        kind: kind::USEITEM,
        quality: 9,
        ..amulet_out()
    });
    let mut fx = with_copy(r);
    let ring = fx.ring;
    fx.log.script().tempered = (21, 34);
    assert!(fx.transmute());
    let copy = fx.output();
    assert_ne!(copy, ring);
    let it = fx.sim.events.sys.hooks.items.get(copy).unwrap();
    assert_eq!(
        (it.quality, it.rare_prefix, it.rare_suffix),
        (q::TEMPERED, 21, 34)
    );
    assert_eq!(fx.sim.events.sys.units.get(copy).unwrap().mode, 0);
    assert!(fx.items_of().contains(&copy));
}

/// `rem` drops the output's runeword stats, then duplicates each item socketed
/// in the source (the inventory model's fillers of the ring: none here, so the
/// copy is the only duplicate, on the model). A socketed source cannot be staged:
/// linking into an item is the move rest's `link_into_item`, which the fixture
/// leaves unanswered.
#[test]
fn rem_drops_the_runeword_stats() {
    let r = output(OutputSlot {
        kind: kind::USEITEM,
        flags: output_flags::REM,
        ..amulet_out()
    });
    let mut fx = with_copy(r);
    let ring = fx.ring;
    assert!(fx.inv().state.fillers(ring).is_empty());
    assert!(fx.transmute());
    let copy = fx.output();
    assert_ne!(copy, ring);
    let log = fx.log.take();
    assert!(log.contains(&format!("runeword {}", copy.0)), "{log:?}");
    no_rest_copy(&log);
    assert!(fx.items_of().contains(&copy));
}

/// `rep` repairs a broken output; `rch` recharges.
#[test]
fn rep_and_rch() {
    let r = output(OutputSlot {
        kind: kind::USEITEM,
        flags: output_flags::REP | output_flags::RCH,
        ..amulet_out()
    });
    let mut fx = with_copy(r);
    // The ring broken: its copy (the output) carries the flags.
    let ring = fx.ring;
    fx.sim.events.sys.hooks.items.get_mut(ring).unwrap().flags |= flag::BROKEN;
    assert!(fx.transmute());
    let copy = fx.output();
    assert_ne!(copy, ring);
    assert_ne!(
        fx.sim.events.sys.hooks.items.get(copy).unwrap().flags & flag::BROKEN,
        0
    );
    let log = fx.log.take();
    assert!(log.contains(&format!("repair {}", copy.0)), "{log:?}");
    assert!(log.contains(&format!("recharge {}", copy.0)), "{log:?}");
}

/// An output without room is freed; a placed `hst ` quest item runs the quest
/// hook.
#[test]
fn placement_outcomes() {
    let r = output(OutputSlot {
        kind: kind::USEITEM,
        ..amulet_out()
    });
    let mut fx = with_copy(r);
    // The copy (of the ring, already in the cube) is wider than the
    // cube's 3 × 4 grid: created, not placed, freed.
    fx.inv().tables.items[RING].invwidth = 4;
    let before = fx.sim.events.sys.hooks.items.len();
    assert!(fx.transmute());
    assert_eq!(fx.sim.events.sys.hooks.items.len(), before - 1, "ring gone");
    assert!(fx.items_of().iter().all(|&u| u != fx.ring));
    assert!(fx.sim.game.lists.unit(fx.ring).is_none());

    let r = output(OutputSlot {
        item: HST as u16,
        ..amulet_out()
    });
    let mut fx = CubeFx::new(2, fields(true), vec![r]);
    let ok = fx.transmute();
    assert!(ok, "{:?}", fx.sim.world.cube.as_ref().unwrap().errors);
    let hst = fx.output();
    let log = fx.log.take();
    assert!(log.contains(&format!("hook {} hst ", hst.0)), "{log:?}");
}

/// Output kind 1: the cow portal decides success; without it nothing is
/// committed.
#[test]
fn cow_portal_decides() {
    let r = output(OutputSlot {
        kind: kind::COW_PORTAL,
        ..amulet_out()
    });
    // The wired host runs the cow-portal check on its quest control, not the
    // pending: this player is in no room (level ≠ 1), so it refuses whatever
    // the pending says; nothing is consumed and sound 20 is staged. The allowed
    // case is covered by d2-sim's `cow_portal_rules`.
    for cow in [false, true] {
        let mut fx = CubeFx::new(2, fields(true), vec![r.clone()]);
        fx.log.script().cow = cow;
        assert!(!fx.transmute());
        let refused = (fx.player, d2_sim::world::cube::SOUND_COW_REFUSED);
        assert!(fx.parts().staged.sounds.contains(&refused));
    }
}

// ---- C→S 0x2A -------------------------------------------------------------------------------

/// 0x2A with `item` into the host's cube; the result and the sounds.
fn put(fx: &mut CubeFx, item: d2_sim::units::UnitId) -> (ResultCode, usize) {
    let mut m = vec![0x2A];
    m.extend(fx.guid(item).to_le_bytes());
    m.extend(fx.guid(fx.cube).to_le_bytes());
    let (r, _) = mutants_handlers_fx::handle(&mut fx.sim, 0, &m);
    (r, fx.parts().staged.sounds.len())
}

/// An item mode above 4 → 1, even in the inventory.
#[test]
fn put_item_mode_above_cursor() {
    let mut fx = CubeFx::new(2, fields(true), vec![]);
    let ring = fx.ring;
    fx.sim.events.sys.units.get_mut(ring).unwrap().mode = 5;
    assert_eq!(put(&mut fx, ring).0, ResultCode::Refused);
}

/// Trading = interaction type 0 with a live unit; then a cube whose page is not 0
/// refuses with sound 19 (result 0). Type 0 with no such unit, or another type,
/// is not trading.
#[test]
fn put_while_trading() {
    let setup = |interaction: fn(u32, u32) -> (u8, u32)| {
        let mut fx = CubeFx::new(2, fields(true), vec![]);
        let (p, c) = (fx.player, fx.cube);
        let pg = fx.guid(p);
        fx.sim.events.sys.hooks.items.get_mut(c).unwrap().inv_page = 4;
        // A second ring on the player's cursor (mode 4, not stored).
        let r = mutants_handlers_fx::new_item(&mut fx.sim, RING, 4);
        fx.sim.events.sys.hooks.items.get_mut(r).unwrap().inv_page = 0;
        fx.inv()
            .state
            .inventories
            .get_mut(&p)
            .unwrap()
            .set_cursor(Some(r));
        let (ty, guid) = interaction(pg, 9999);
        let rec = fx.sim.events.sys.units.get_mut(p).unwrap();
        rec.interact.reset();
        rec.interact.set(ty, guid);
        let out = put(&mut fx, r);
        let page = fx.sim.events.sys.hooks.items.get(r).unwrap().inv_page;
        (out, page)
    };
    assert_eq!(setup(|p, _| (0, p)), ((ResultCode::Done, 1), 0));
    assert_eq!(setup(|_, none| (0, none)), ((ResultCode::Done, 0), 3));
    assert_eq!(setup(|p, _| (4, p)), ((ResultCode::Done, 0), 3));
}

/// `sock` with quantity 3 on a normal output whose maximum is 4
/// (min(`gemsockets`, `MaxSock1`) at ilvl ≤ 25; a 2×2 item) → flag 0x800 and
/// 3 sockets (stat 194).
#[test]
fn sock_output() {
    let r = output(OutputSlot {
        flags: output_flags::SOCK,
        quantity: 3,
        ..amulet_out()
    });
    let mut fx = CubeFx::new(2, fields(true), vec![r]);
    let rec = &mut fx.sim.world.tables.items[AMULET];
    (rec.gemsockets, rec.invwidth, rec.invheight) = (4, 2, 2);
    fx.sim.world.tables.itemtypes[usize::from(mutants_handlers_fx::T_AMULET)].maxsock1 = 4;
    assert!(fx.transmute());
    let amu = fx.output();
    assert_ne!(
        fx.sim.events.sys.hooks.items.get(amu).unwrap().flags & flag::SOCKETED,
        0
    );
    let g = &mut fx.sim;
    let sockets = g.events.with(&mut g.game, |_, v| v.stat(amu, 194));
    assert_eq!(sockets, 3);
}
