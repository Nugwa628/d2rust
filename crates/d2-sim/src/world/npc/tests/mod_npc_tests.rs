//! A mod game's NPC roles and menu actions (`docs/plans/mods/seams.md`
//! MS8c), with a synthetic provider: class 527 (an interact class with no
//! D2RUST role) trades and heals, Charsi loses her store, and menu actions
//! 7 and 8 are the mod's own.

use std::sync::Arc;

use super::*;
use crate::rules::{ModRules, NpcProvider, Ruleset};
use crate::world::npc::{NpcHost, NpcRoles};

/// The mod's NPC class (an `interact` row with no D2RUST role).
const MOD_NPC: u16 = 527;

#[derive(Debug)]
struct TestNpcs;

impl NpcProvider for TestNpcs {
    fn roles(&self, class: u16) -> Option<NpcRoles> {
        match class {
            MOD_NPC => Some(NpcRoles {
                trades: true,
                heals: true,
                ..NpcRoles::default()
            }),
            class::CHARSI => Some(NpcRoles {
                trades: false,
                ..NpcRoles::d2rust(class)
            }),
            _ => None,
        }
    }

    fn menu_action(
        &self,
        _ctl: &mut NpcControl,
        w: &mut dyn NpcHost,
        player: UnitId,
        npc: UnitId,
        class: u16,
        action: u32,
        item: u32,
    ) -> Option<Result<u32, NpcError>> {
        match action {
            7 => {
                let g = w.guid(npc) as u8;
                w.send(player, &[0xEE, g, class as u8, item as u8]);
                Some(Ok(0))
            }
            8 => Some(Err(NpcError::Table("mod action 8".into()))),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct TestRules(TestNpcs);

impl ModRules for TestRules {
    fn name(&self) -> &str {
        "npc test mod"
    }
    fn npcs(&self) -> Option<&dyn NpcProvider> {
        Some(&self.0)
    }
}

fn modded() -> Fake {
    let mut w = Fake::new();
    w.rules = Ruleset::Mod(Arc::new(TestRules(TestNpcs)));
    w
}

#[test]
fn roles_come_from_the_mod_else_d2rust() {
    let m = Ruleset::Mod(Arc::new(TestRules(TestNpcs)));
    let o = Ruleset::Original;
    assert_eq!(NpcRoles::of(&o, MOD_NPC), NpcRoles::default());
    assert!(NpcRoles::of(&m, MOD_NPC).trades && NpcRoles::of(&m, MOD_NPC).heals);
    assert!(NpcRoles::of(&o, class::CHARSI).trades);
    assert!(!NpcRoles::of(&m, class::CHARSI).trades);
    // A class the mod leaves alone keeps D2RUST's roles.
    assert_eq!(
        NpcRoles::of(&m, class::GHEED),
        NpcRoles::d2rust(class::GHEED)
    );
    assert!(NpcRoles::of(&m, class::GHEED).gambles);
}

#[test]
fn the_mod_npc_trades_and_charsi_does_not() {
    for (mut w, mod_trades) in [(Fake::new(), false), (modded(), true)] {
        let mut c = control(0);
        let npc = w.npc(MOD_NPC, 20);
        talk(&mut c, &mut w, npc);
        assert_eq!(c.menu_action(&mut w, PLAYER, &msg38(1, 20, 0)).unwrap(), 0);
        assert_eq!(w.has("trade 1020 single true gamble false"), mod_trades);
        let charsi = w.npc(class::CHARSI, 6);
        talk(&mut c, &mut w, charsi);
        c.menu_action(&mut w, PLAYER, &msg38(1, 6, 0)).unwrap();
        assert_eq!(w.has("trade 1006 single true gamble false"), !mod_trades);
    }
}

#[test]
fn the_mod_npc_heals() {
    for (mut w, heals) in [(Fake::new(), false), (modded(), true)] {
        let mut c = control(0);
        w.npc(MOD_NPC, 20);
        w.set(PLAYER, stat::LIFE, 40);
        c.interact(&mut w, PLAYER, &msg13(20)).unwrap();
        w.log.clear();
        c.chat_open(&mut w, PLAYER, &msg9(0x2F, 20));
        assert_eq!(w.has("send stat 6 100"), heals);
    }
}

#[test]
fn mod_menu_actions_run_first() {
    let mut c = control(0);
    let mut w = modded();
    let charsi = w.npc(class::CHARSI, 6);
    talk(&mut c, &mut w, charsi);
    assert_eq!(c.menu_action(&mut w, PLAYER, &msg38(7, 6, 9)).unwrap(), 0);
    assert_eq!(w.sent, [vec![0xEE, 6, class::CHARSI as u8, 9]]);
    assert!(matches!(
        c.menu_action(&mut w, PLAYER, &msg38(8, 6, 0)),
        Err(NpcError::Table(_))
    ));
    // `None` runs D2RUST's action: the hire list at Kashya.
    w.sent.clear();
    let kashya = w.npc(class::KASHYA, 12);
    talk(&mut c, &mut w, kashya);
    c.menu_action(&mut w, PLAYER, &msg38(3, 12, 0)).unwrap();
    assert_eq!(w.ids()[0], 0x4F);
    assert_eq!(w.sent.len(), 11);
    // Under `Original` action 7 does nothing.
    let mut o = Fake::new();
    let charsi = o.npc(class::CHARSI, 6);
    talk(&mut c, &mut o, charsi);
    assert_eq!(c.menu_action(&mut o, PLAYER, &msg38(7, 6, 9)).unwrap(), 0);
    assert!(o.sent.is_empty() && o.log.is_empty());
}

#[test]
fn d2rust_checks_run_before_the_mod() {
    let mut c = control(0);
    let mut w = modded();
    let charsi = w.npc(class::CHARSI, 6);
    talk(&mut c, &mut w, charsi);
    let m = msg38(8, 6, 0);
    // Size, unit check, no interaction list: the mod's fatal 8 never runs.
    assert_eq!(c.menu_action(&mut w, PLAYER, &m[..12]).unwrap(), 3);
    w.unit_check = 2;
    assert_eq!(c.menu_action(&mut w, PLAYER, &m).unwrap(), 2);
    w.unit_check = 0;
    w.units.get_mut(&charsi).unwrap().interaction = None;
    assert_eq!(c.menu_action(&mut w, PLAYER, &m).unwrap(), 0);
    // A class without `interact` never reaches the mod.
    w.npc(5, 30);
    assert_eq!(c.menu_action(&mut w, PLAYER, &msg38(8, 30, 0)).unwrap(), 0);
}
