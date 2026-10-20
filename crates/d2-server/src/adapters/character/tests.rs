//! Tests of `adapters::character`: the load order and the pure load rules
//! (header fields, quest normalisation, gold limits, golem check) against a
//! recording [`CharacterWorld`].

use std::collections::BTreeMap;

use d2_formats::d2s::{
    Body, Corpse, D2s, Golem, Header, Hireling, ItemEntry, Quests, Slot, StatEntry, Stats,
};

use super::*;

/// Records every call; stats live in a map (totals = base).
#[derive(Default)]
struct Rec {
    log: Vec<String>,
    stats: BTreeMap<u16, i32>,
    skill_90: bool,
    start_skill: u16,
    has_start: bool,
    /// Steps answered as unapplied.
    missing: Vec<&'static str>,
}

impl Rec {
    fn step(&mut self, s: &'static str, line: String) -> Result<(), Unapplied> {
        if self.missing.contains(&s) {
            return Err(Unapplied {
                step: s,
                reason: "test",
            });
        }
        self.log.push(line);
        Ok(())
    }
}

impl CharacterWorld for Rec {
    fn player_skills(&mut self) -> Result<(), Unapplied> {
        self.step("player_skills", "player skills".into())
    }
    fn new_character_setup(&mut self) -> Result<(), Unapplied> {
        self.step("setup", "setup".into())
    }
    fn start_stats(&mut self, act: u8) -> Result<(), Unapplied> {
        self.step("start_stats", format!("start stats {act}"))
    }
    fn start_items(&mut self) -> Result<(), Unapplied> {
        self.step("start_items", "start items".into())
    }
    fn start_skill(&self) -> Result<u16, Unapplied> {
        Ok(self.start_skill)
    }
    fn has_skill(&self, skill: u16) -> Result<bool, Unapplied> {
        Ok(if skill == SKILL_IRON_GOLEM {
            self.skill_90
        } else {
            self.has_start
        })
    }
    fn set_mouse_skills(&mut self, right: Option<u16>) -> Result<(), Unapplied> {
        self.step("mouse", format!("mouse {right:?}"))
    }
    fn quest_entry(&mut self, mode: u8) -> Result<(), Unapplied> {
        self.step("quest_entry", format!("quest entry {mode}"))
    }
    fn apply_header(&mut self, h: &HeaderLoad) -> Result<(), Unapplied> {
        self.step("header", format!("header act {}", h.act))
    }
    fn set_quests(&mut self, r: &[[u8; 96]; 3]) -> Result<(), Unapplied> {
        self.step("quests", format!("quests {:02x}{:02x}", r[0][0], r[0][1]))
    }
    fn set_waypoints(&mut self, _: &[[u8; 16]; 3]) -> Result<(), Unapplied> {
        self.step("waypoints", "waypoints".into())
    }
    fn set_npc_fields(&mut self, a: &[[u8; 8]; 3], _: &[[u8; 8]; 3]) -> Result<(), Unapplied> {
        self.step("npc", format!("npc {:02x}", a[0][0]))
    }
    fn set_base_stat(&mut self, id: u16, layer: u16, value: i32) -> Result<(), Unapplied> {
        self.stats.insert(id, value);
        self.step("stat", format!("stat {id}.{layer} {value}"))
    }
    fn stat(&self, id: u16) -> Result<i32, Unapplied> {
        Ok(self.stats.get(&id).copied().unwrap_or(0))
    }
    fn add_skill_level(&mut self, index: usize, level: u8) -> Result<(), Unapplied> {
        self.step("skill", format!("skill {index} +{level}"))
    }
    fn create_items(&mut self, list: ItemList, e: &[ItemEntry]) -> Result<(), Unapplied> {
        self.step("items", format!("items {list:?} {}", e.len()))
    }
    fn create_corpse(&mut self, _: &Corpse) -> Result<(), Unapplied> {
        self.step("corpse", "corpse".into())
    }
    fn restore_hireling(&mut self, b: &Hireling) -> Result<(), Unapplied> {
        self.step("hireling", format!("hireling {}", b.is_present()))
    }
    fn hireling_items_loaded(&mut self) -> Result<(), Unapplied> {
        self.step("hireling_rule8", "hireling rule 8".into())
    }
    fn resolve_item_indices(&mut self) -> Result<(), Unapplied> {
        self.step("indices", "indices".into())
    }
    fn next_exp(&self, level: i32) -> Result<i32, Unapplied> {
        Ok(level * 1000)
    }
}

fn st(id: u16, value: i32) -> StatEntry {
    StatEntry {
        id,
        layer: 0,
        value,
    }
}

fn item() -> ItemEntry {
    ItemEntry {
        bytes: vec![0x4A, 0x4D, 0x10, 0x20, 0xA0, 0x00],
    }
}

fn full_save() -> D2s {
    let mut h = Header {
        towns: [0x82, 0, 0],
        map_seed: 0xABCD,
        ..Header::default()
    };
    h.hireling.seed = 5;
    let mut quests = Quests::default();
    quests.records[0][0] = 0b0000_0010; // slot 0 bit 1
    quests.records[0][1] = 0b0110_0000; // bits 13, 14
    D2s {
        header: h,
        body: Some(Body {
            quests,
            stats: Stats::Bits(vec![
                st(6, 0x1000),
                st(8, 0x800),
                st(10, 1),
                st(11, 0x5000),
                st(12, 3),
                st(14, 40_000),
            ]),
            skills: {
                let mut s = vec![0u8; 30];
                s[2] = 4;
                s
            },
            items: vec![item(), item()],
            corpses: vec![Corpse {
                items: vec![item()],
                ..Corpse::default()
            }],
            hireling_items: Some(Some(vec![item()])),
            golem: Some(Golem {
                flag: 1,
                item: Some(item()),
            }),
            ..Body::default()
        }),
    }
}

// A full save runs every step in the fixed load order, post-load values last.
#[test]
fn full_save_runs_the_steps_in_the_masters_order() {
    let mut w = Rec {
        skill_90: true,
        ..Rec::default()
    };
    let r = load(&full_save(), &LoadContext::default(), &mut w).unwrap();
    assert!(!r.new_character);
    assert_eq!(r.act, 2);
    assert!(r.unapplied.is_empty());
    let want = [
        // The player init's native skills.
        "player skills",
        "header act 2",
        // Slot 0: bit 1 kept, bit 15 set; bits 13, 14 cleared.
        "quests 0280",
        "waypoints",
        "npc 00",
        "stat 6.0 4096",
        "stat 8.0 2048",
        "stat 10.0 1",
        "stat 11.0 20480",
        "stat 12.0 3",
        "stat 14.0 40000",
        "skill 2 +4",
        "items Player 2",
        "corpse",
        "items Corpse 1",
        "hireling true",
        "items Hireling 1",
        "hireling rule 8",
        "items Golem 1",
        // Post-load: gold 40000 > 3 × 10000 → 0; stamina := max; item
        // indices; hitpoints / mana remembered; 67–69; nextexp.
        "stat 14.0 0",
        "stat 10.0 20480",
        "indices",
        "stat 6.0 4096",
        "stat 8.0 2048",
        "stat 67.0 100",
        "stat 68.0 100",
        "stat 69.0 100",
        "stat 30.0 3000",
    ];
    assert_eq!(w.log, want);
}

// A golem item is loaded only when the player has the Iron Golem skill.
#[test]
fn golem_item_needs_the_iron_golem_skill() {
    let mut w = Rec::default();
    assert_eq!(
        load(&full_save(), &LoadContext::default(), &mut w),
        Err(LoadError::GolemSkill)
    );
    // g = 0: no item, no check.
    let mut s = full_save();
    s.body.as_mut().unwrap().golem = Some(Golem::default());
    let mut w = Rec::default();
    assert!(load(&s, &LoadContext::default(), &mut w).is_ok());
    assert!(!w.log.iter().any(|l| l.contains("Golem")));
}

// A stub save runs the new-character start and reads no stat, skill or item section.
#[test]
fn stub_starts_a_new_character() {
    let stub = D2s::new_stub(b"Neo", 2, 0, 1).unwrap();
    let mut w = Rec {
        start_skill: 70,
        has_start: true,
        ..Rec::default()
    };
    let r = load(&stub, &LoadContext::default(), &mut w).unwrap();
    assert!(r.new_character);
    assert_eq!(r.act, 0);
    assert_eq!(
        w.log,
        [
            "player skills",
            "setup",
            "start stats 0",
            "start items",
            "mouse Some(70)",
            "quest entry 1"
        ]
    );
    // StartSkill 0, or the unit lacks it: no right skill.
    let mut w = Rec {
        start_skill: 70,
        ..Rec::default()
    };
    load(&stub, &LoadContext::default(), &mut w).unwrap();
    assert_eq!(w.log[4], "mouse None");
    // No stat, skill or item section is read.
    assert!(!w.log.iter().any(|l| l.starts_with("stat")));
}

// Steps the provider cannot apply are reported in order and the load goes on.
#[test]
fn unapplied_steps_are_reported_in_order() {
    let mut w = Rec {
        skill_90: true,
        missing: vec!["waypoints", "corpse", "indices"],
        ..Rec::default()
    };
    let r = load(&full_save(), &LoadContext::default(), &mut w).unwrap();
    let steps: Vec<_> = r.unapplied.iter().map(|u| u.step).collect();
    assert_eq!(steps, ["waypoints", "corpse", "indices"]);
    // The later steps still ran.
    assert!(w.log.contains(&"items Golem 1".to_string()));
}

// The header's client fields: act from the town byte, map seed, hotkeys and mouse skills.
#[test]
fn header_fields() {
    let mut h = Header {
        weapon_switch: 0xFF,
        status: 0x28,
        create_time: 77,
        towns: [0x01, 0x83, 0x05],
        map_seed: 99,
        client_cf: 4,
        ..Header::default()
    };
    h.hotkeys[0] = Slot::encode(36, true, 2).unwrap();
    h.mouse[1] = Slot::encode(0, false, 0).unwrap();
    let ctx = |difficulty, map_seed_applies| LoadContext {
        difficulty,
        map_seed_applies,
    };
    let l = header_load(&h, &ctx(1, true));
    assert!(l.weapon_switch);
    assert_eq!((l.status, l.create_time, l.client_480), (0x28, 77, 4));
    assert_eq!(l.act, 3);
    assert_eq!(l.map_seed, Some(99));
    assert_eq!(
        l.hotkeys[0],
        SkillSlot {
            skill: 36,
            left: true,
            item: 2
        }
    );
    assert_eq!(
        l.hotkeys[1],
        SkillSlot {
            skill: -1,
            left: false,
            item: -1
        }
    );
    assert_eq!(
        l.mouse[1],
        SkillSlot {
            skill: 0,
            left: false,
            item: -1
        }
    );
    // No 0x80 or the game condition off: no map seed.
    assert_eq!(header_load(&h, &ctx(0, true)).map_seed, None);
    assert_eq!(header_load(&h, &ctx(1, false)).map_seed, None);
    // Act ≥ 5 → 0.
    assert_eq!(header_load(&h, &ctx(2, true)).act, 0);
    assert_eq!(header_load(&h, &ctx(0, true)).act, 1);
}

// Quest records are normalised per slot; bytes past the 42 slots are kept.
#[test]
fn quest_normalisation() {
    let mut rec = [0u8; 96];
    // Slot 0: bits 1, 13, 14 → 1, 15. Slot 41: bit 14 only → 0.
    rec[0..2].copy_from_slice(&0x6002u16.to_le_bytes());
    rec[82..84].copy_from_slice(&0x4001u16.to_le_bytes());
    // Past the 42 slots: kept.
    rec[84..86].copy_from_slice(&0x6002u16.to_le_bytes());
    let n = normalise_quests(&rec);
    assert_eq!(u16::from_le_bytes([n[0], n[1]]), 0x8002);
    assert_eq!(u16::from_le_bytes([n[82], n[83]]), 0x0001);
    assert_eq!(u16::from_le_bytes([n[84], n[85]]), 0x6002);
}

// Gold and stash limits at their boundaries.
#[test]
fn gold_limit_vectors() {
    assert_eq!(gold_limits(30_000, 2_500_000, 3), (30_000, 2_500_000));
    assert_eq!(gold_limits(30_001, 2_500_001, 3), (0, 0));
    assert_eq!(gold_limits(-1, -1, 3), (0, 0));
}

// A loaded item record's flags are adjusted by `loaded_flags`.
#[test]
fn loaded_item_flags() {
    assert_eq!(loaded_flags(0x00A0_2010), 0x00A8_0010);
}

// A load error maps to its load result code.
#[test]
fn load_errors_map_to_their_results() {
    // Internal 23 (golem) → result 10.
    assert_eq!(LoadError::GolemSkill.result(), 10);
}
