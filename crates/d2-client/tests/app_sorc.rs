//! The Sorceress's skills cast on the play host, headless, on the user's
//! install (`real_rig`; q-fixture-migrate): a new sorceress leaves the
//! camp (Fire Wall and Teleport refuse in town), learns the skill, and
//! C→S 0x0C casts it at a point → the server's skill use → the body's
//! effect (missiles, states, the teleport). One test per skill. Skill
//! ids come from the install's `skills.txt` by name; costs, states,
//! missile links and counts from the same rows.

use crate::app_support;
use crate::real_rig;

use std::collections::BTreeSet;

use d2_client::bridge::BridgeResource;
use d2_data::bin::TableFiles;
use d2_sim::units::UnitType;
use real_rig::{skill_row, Rig};

/// The install's `skills.txt` row of the skill called `name`: its
/// `(Id, Param1)`.
fn txt_row(name: &str) -> (usize, i32) {
    let d = app_support::live();
    let (_, bytes) = d
        .archives
        .read_excel("skills.txt")
        .expect("skills.txt reads")
        .expect("the install has skills.txt");
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next().expect("header").split('\t').collect();
    let col = |n: &str| {
        head.iter()
            .position(|h| *h == n)
            .unwrap_or_else(|| panic!("{n}"))
    };
    let (c_name, c_id, c_p1) = (col("skill"), col("Id"), col("Param1"));
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        if f.get(c_name) == Some(&name) {
            let id = f[c_id].parse().expect("Id");
            return (id, f[c_p1].parse().unwrap_or(0));
        }
    }
    panic!("no skills.txt row called {name}");
}

/// The install's skill id of `name`.
fn skill(name: &str) -> usize {
    txt_row(name).0
}

/// A new sorceress with `names` learned, out of town and strengthened
/// (`Rig::strengthen`).
fn game(names: &[&str]) -> Rig {
    let ids: Vec<usize> = names.iter().map(|n| skill(n)).collect();
    let mut r = Rig::new("sorceress", &ids);
    r.leave_town();
    r.strengthen();
    r
}

/// What a cast showed over its frames.
struct Cast {
    /// The most missiles alive at once.
    most: usize,
    /// The largest one-frame fall of the mana in the cast, and the
    /// largest one-frame rise at rest before it (regeneration), 1/256.
    drop: i32,
    regen: i32,
    /// The modes the client's local player was in.
    client_modes: BTreeSet<u32>,
    /// The state bits the client's local player had.
    client_states: BTreeSet<u8>,
    /// Whether a message with the byte 0x4D reached the client during the
    /// cast (the old rigs' loose S→C 0x4D check).
    saw_4d: bool,
}

/// The client model's local player: its mode and state bits.
fn client_player(r: &Rig) -> Option<(u32, BTreeSet<u8>)> {
    let w = r.app.world().resource::<BridgeResource>().0.world();
    let u = w.units.get(&w.local_player?)?;
    Some((u.mode, u.states.clone()))
}

/// Makes `id` the right skill and casts it at the point 6 sub-tiles east
/// (C→S 0x0C), then runs `frames` frames.
fn cast(r: &mut Rig, id: usize, frames: usize) -> Cast {
    r.select_right(id);
    // Regeneration at rest, from half mana: the cast's fall is the cost
    // less what a frame regenerates.
    r.with(|sim, p| {
        sim.events
            .action
            .with(&mut sim.game, |_, v| v.set_base(p, 8, 50 << 8));
    });
    let mut regen = 0;
    let mut prev = r.mana();
    for _ in 0..10 {
        r.step(1);
        let m = r.mana();
        regen = regen.max(m - prev);
        prev = m;
    }
    let mark = r.s2c_mark();
    r.right_click_point(6, 0);
    let mut c = Cast {
        most: 0,
        drop: 0,
        regen,
        client_modes: BTreeSet::new(),
        client_states: BTreeSet::new(),
        saw_4d: false,
    };
    for _ in 0..frames {
        r.step(1);
        c.most = c
            .most
            .max(r.with(|sim, _| sim.game.lists.units_of_type(UnitType::Missile).len()));
        let m = r.mana();
        c.drop = c.drop.max(prev - m);
        prev = m;
        if let Some((m, s)) = client_player(r) {
            c.client_modes.insert(m);
            c.client_states.extend(s);
        }
    }
    c.saw_4d = r.s2c_contains_since(mark, 0x4D);
    c
}

/// The mana a cast of `id` at skill level 1 costs (
/// `(mana + max(L − 1, 0) × lvlmana) << manashift`, 1/256 mana).
fn cost(id: usize) -> i32 {
    let row = skill_row(id);
    i32::from(row.mana) << row.manashift
}

/// The cast paid `cost`: one frame fell by it less that frame's
/// regeneration.
fn paid(c: &Cast, cost: i32) -> bool {
    (cost - c.regen..=cost).contains(&c.drop)
}

/// How long a cast runs here: past the animation and the missile's life.
const FRAMES: usize = 60;

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn fire_bolt_and_ice_bolt_cost_mana_and_fly() {
    for name in ["Fire Bolt", "Ice Bolt"] {
        let id = skill(name);
        let row = skill_row(id);
        let mut r = game(&[name]);
        let c = cast(&mut r, id, FRAMES);
        let e = r.errors();
        assert!(
            paid(&c, cost(id)),
            "{name}: mana; drop {} cost {} regen {}; {e}",
            c.drop,
            cost(id),
            c.regen
        );
        assert!(c.most > 0, "{name}: missile; {e}");
        assert!(c.saw_4d, "{name}: 0x4D");
        assert!(
            c.client_modes.contains(&u32::from(row.anim)),
            "{name}: the cast mode reaches the client: {:?}; {e}",
            c.client_modes
        );
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn charged_bolt_makes_its_bolts() {
    let (id, bolts) = txt_row("Charged Bolt");
    let mut r = game(&["Charged Bolt"]);
    let c = cast(&mut r, id, FRAMES);
    let e = r.errors();
    assert!(
        paid(&c, cost(id)),
        "mana; drop {} cost {} regen {}; {e}",
        c.drop,
        cost(id),
        c.regen
    );
    // `calc1` = `min(24, ln12)`; `ln12` is Param1 at level 1.
    assert!(
        c.most >= bolts as usize,
        "{bolts} bolts, got {}; {e}",
        c.most
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn frozen_armor_sets_its_state() {
    let id = skill("Frozen Armor");
    let state = skill_row(id).aurastate;
    let mut r = game(&["Frozen Armor"]);
    let me = r.player();
    let c = cast(&mut r, id, FRAMES);
    let e = r.errors();
    assert!(r.state_on(me, state), "state; {e}");
    // q-states-auras: the toggle reaches the client (S→C 0xA8, the state).
    assert!(
        c.client_states.contains(&(state as u8)),
        "the state reaches the client: {:?}; {e}",
        c.client_states
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn enchant_sets_its_state() {
    let id = skill("Enchant");
    let state = skill_row(id).aurastate;
    let mut r = game(&["Enchant"]);
    let me = r.player();
    cast(&mut r, id, FRAMES);
    assert!(r.state_on(me, state), "state; {}", r.errors());
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn teleport_moves_the_caster() {
    let id = skill("Teleport");
    let mut r = game(&["Teleport"]);
    let from = r.pos();
    cast(&mut r, id, FRAMES);
    let to = r.pos();
    assert_ne!(from, to, "moved; {}", r.errors());
    assert!(
        from.0.abs_diff(to.0) <= 8,
        "to the point, not across the map"
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn nova_makes_a_ring() {
    let id = skill("Nova");
    let mut r = game(&["Nova"]);
    let c = cast(&mut r, id, FRAMES);
    // The ring is 64 missiles.
    assert!(c.most >= 64, "ring, got {}; {}", c.most, r.errors());
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn blizzard_and_meteor_make_a_missile_at_the_point() {
    for name in ["Blizzard", "Meteor"] {
        let id = skill(name);
        let mut r = game(&[name]);
        let c = cast(&mut r, id, FRAMES);
        assert!(c.most > 0, "{name}: missile; {}", r.errors());
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn fire_wall_makes_its_wall() {
    let id = skill("Fire Wall");
    let row = skill_row(id);
    let mut r = game(&["Fire Wall"]);
    let c = cast(&mut r, id, FRAMES);
    // Two side missiles, plus the centre one when `srvmissileb` is set.
    let want = 2 + usize::from(row.srvmissileb != 0xFFFF);
    assert!(
        c.most >= want,
        "wall of {want}, got {}; {}",
        c.most,
        r.errors()
    );
}
