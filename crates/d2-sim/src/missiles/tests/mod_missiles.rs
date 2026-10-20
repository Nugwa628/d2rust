//! A mod game's missile functions (`docs/plans/mods/seams.md` MS6): its
//! server-do and server-hit bodies answer before D2RUST's, past D2RUST's
//! tables and in place of its own, also where one D2RUST body calls
//! another; without an answer D2RUST's run.

use std::sync::{Arc, Mutex};

use super::*;
use crate::rules::{MissileProvider, MissileSlot, ModRules, Ruleset};

/// Slots past D2RUST's tables (server-do 1-52, server-hit 1-70).
const NEW_DO: i16 = 60;
const NEW_HIT: i16 = 80;

/// The mod's missiles: server-do `NEW_DO` removes the missile, server-hit
/// `NEW_HIT` adds result bit 4 (the hit handler stops and returns 1
/// instead of 2); the indices in `replace` answer the same way for
/// D2RUST's slots, the others are left to D2RUST. Every call is logged.
#[derive(Debug, Default)]
struct ModMissiles {
    replace_do: Vec<i16>,
    replace_hit: Vec<i16>,
    log: Mutex<Vec<String>>,
}

impl ModMissiles {
    fn note(&self, s: String) {
        self.log.lock().unwrap().push(s);
    }
    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl MissileProvider for ModMissiles {
    fn has(&self, slot: MissileSlot, index: i16) -> bool {
        match slot {
            MissileSlot::Do => index == NEW_DO,
            MissileSlot::Hit => index == NEW_HIT,
        }
    }
    fn srv_do(
        &self,
        _game: &mut Game,
        cx: &mut Ctx<'_, dyn MissileWorld + '_>,
        index: i16,
        m: UnitId,
    ) -> Option<i32> {
        self.note(format!("do {index}"));
        // The body reaches D2RUST's world through the `dyn` context.
        let _ = cx.world.position(m);
        (index == NEW_DO || self.replace_do.contains(&index)).then_some(2)
    }
    fn srv_hit(
        &self,
        _game: &mut Game,
        _cx: &mut Ctx<'_, dyn MissileWorld + '_>,
        index: i16,
        _m: UnitId,
        _unit: Option<UnitId>,
        c: i32,
    ) -> Option<i32> {
        self.note(format!("hit {index}"));
        (index == NEW_HIT || self.replace_hit.contains(&index)).then_some(c | 4)
    }
}

impl ModRules for ModMissiles {
    fn name(&self) -> &str {
        "mod-missiles"
    }
    fn missiles(&self) -> Option<&dyn MissileProvider> {
        Some(self)
    }
}

fn install_mod(w: &mut World, replace_do: &[i16], replace_hit: &[i16]) -> Arc<ModMissiles> {
    let m = Arc::new(ModMissiles {
        replace_do: replace_do.to_vec(),
        replace_hit: replace_hit.to_vec(),
        ..ModMissiles::default()
    });
    w.fake.rules = Ruleset::Mod(m.clone());
    m
}

fn world(r: MissileRow) -> (World, UnitId) {
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    (w, m)
}

#[test]
fn original_ignores_slots_past_the_tables() {
    let mut r = row();
    r.psrvdofunc = NEW_DO as u16;
    r.psrvhitfunc = NEW_HIT as u16;
    r.range = 1;
    let (mut w, m) = world(r);
    for _ in 0..3 {
        w.frame();
    }
    assert!(w.alive(m));
    assert_eq!(w.hit(m, None, true), 2);
    assert!(w.store.unhandled.is_empty());
}

#[test]
fn mod_slots_past_the_tables_run() {
    let mut r = row();
    r.psrvdofunc = NEW_DO as u16;
    r.psrvhitfunc = NEW_HIT as u16;
    let (mut w, m) = world(r);
    let p = install_mod(&mut w, &[], &[]);
    assert_eq!(w.hit(m, None, true), 1);
    w.frame();
    assert!(!w.alive(m));
    assert_eq!(p.log(), ["hit 80", "do 60"]);
    assert!(w.store.unhandled.is_empty());
}

#[test]
fn mod_bodies_replace_and_fall_back() {
    // Server-do 36 (Baal FX control) refreshes the room. A mod that
    // leaves it to D2RUST still sees the call; one that replaces it
    // removes the missile instead.
    let mut r = row();
    r.psrvdofunc = 36;
    let (mut w, m) = world(r.clone());
    let p = install_mod(&mut w, &[], &[]);
    w.frame();
    assert_eq!((w.alive(m), w.fake.logged("refresh")), (true, 1));
    assert_eq!(p.log(), ["do 36"]);

    let (mut w, m) = world(r);
    install_mod(&mut w, &[36], &[]);
    w.frame();
    assert_eq!((w.alive(m), w.fake.logged("refresh")), (false, 0));
}

#[test]
fn bodies_calling_other_slots_go_through_the_mod() {
    // Server-do 34 (Baal taunt control) runs server-do 3 while quiet:
    // a mod's server-do 3 answers there.
    let mut r = row();
    r.psrvdofunc = 34;
    r.submissile1 = 0;
    r.param1 = 100;
    r.param2 = 1;
    let (mut w, m) = world(r);
    let p = install_mod(&mut w, &[3], &[]);
    w.frame();
    assert!(!w.alive(m));
    assert_eq!(p.log(), ["do 34", "do 3"]);

    // Server-hit 3 (potions on the ground) runs server-hit 44.
    let mut r = row();
    r.psrvhitfunc = 3;
    let (mut w, m) = world(r);
    let p = install_mod(&mut w, &[], &[44]);
    assert_eq!(w.hit(m, None, true), 1);
    assert_eq!(p.log(), ["hit 3", "hit 44"]);
}
