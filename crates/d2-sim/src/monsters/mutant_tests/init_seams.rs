//! The default bodies of [`InitHost`]: "the narrowest reading, nothing
//! happens, or the value that makes init do nothing" (`init/seams.rs`).
//! A host with only the required methods answers every defaulted query
//! with that value, and init run on it does nothing past those answers.
use super::*;

/// A host providing only the required methods.
struct Bare {
    game: Game,
    units: Units,
    store: MonsterStore,
}

impl Bare {
    fn new() -> Self {
        Self {
            game: Game::new(),
            units: Units::new(),
            store: MonsterStore::new(),
        }
    }
}

impl InitHost for Bare {
    fn game(&mut self) -> &mut Game {
        &mut self.game
    }
    fn units(&mut self) -> &mut Units {
        &mut self.units
    }
    fn monsters(&mut self) -> &mut MonsterStore {
        &mut self.store
    }
    fn info(&self) -> GameInfo {
        GameInfo::default()
    }
    fn stat(&self, _: UnitId, _: u16) -> i32 {
        0
    }
    fn set_stat(&mut self, _: UnitId, _: u16, _: i32) {}
}

#[test]
fn default_queries_answer_nothing() {
    let mut h = Bare::new();
    let u = UnitId(7);
    assert_eq!(h.level_id(u), 0);
    assert_eq!(h.region_variant_count(u, 3), 0);
    for i in [0, 1, 15] {
        assert_eq!(h.region_variant(u, 3, i), [0; 16]);
    }
    assert!(!h.has_inventory(u));
    for loc in [0, 1, 4, 10] {
        assert!(!h.has_item_at(u, loc), "loc {loc}");
    }
    assert!(h.minions(u).is_empty());
    // Umod 34's alignment gate: 0, run.
    assert_eq!(h.alignment(u), 0);
    assert!(h.minion_owner(u).is_none() && !h.has_state(u, 54));
}

#[test]
fn default_creation_seams_refuse() {
    let mut h = Bare::new();
    let reqs = [
        CreateRequest::default(),
        CreateRequest {
            class: 5,
            x: 100,
            y: -100,
            spread: 4,
            ..CreateRequest::default()
        },
    ];
    for req in &reqs {
        assert_eq!(h.place(req), None);
        assert_eq!(h.allocate(req, req.x, req.y), None);
        assert_eq!(h.boss_spawn(req, Some(3), true), None);
        assert_eq!(h.spawn_with_guid(req), None);
    }
    assert_eq!(h.spawn_boss_minion(UnitId(1), 2, Some(0)), None);
    assert_eq!(h.spawn_boss_minion(UnitId(1), 2, None), None);
}

#[test]
fn default_montype_is_plain_equality() {
    let mut h = Bare::new();
    for (m, t, want) in [
        (0, 0, true),
        (1, 1, true),
        (37, 37, true),
        (2, 1, false),
        (1, 2, false),
        (0xFFFF, 0xFFFF, true),
        (0, 0xFFFF, false),
    ] {
        assert_eq!(h.montype_is(m, t), want, "{m} {t}");
    }
}

#[test]
fn create_on_the_default_seams_places_nothing() {
    // No placement point: creation stops before allocation, and nothing
    // is written to the unit records or the monster store.
    let cx = fake(vec![mon(1, 1, 1, 1)]).cx;
    let mut h = Bare::new();
    let req = CreateRequest {
        class: 0,
        mode: 1,
        x: 10,
        y: 20,
        ..CreateRequest::default()
    };
    assert_eq!(create(&cx, &mut h, &req), None);
    // A probe (flag 0x01) with no point fails too, instead of "placed".
    let probe = CreateRequest {
        flags: create_flag::PROBE,
        ..req
    };
    assert_eq!(create(&cx, &mut h, &probe), None);
    assert!(h.game.lists.units_of_type(UnitType::Monster).is_empty());
    assert!(h.store.get(UnitId(0)).is_none());
}
